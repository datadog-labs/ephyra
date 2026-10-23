// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! bynfuzz <build-dir> <seed> <seconds> <out-dir>
//!
//! One search process. Per case:
//!   1. random bytes -> `wasm-opt -ttf <features> --translate-to-exnref` -> a.wasm
//!      (wasmtime has no legacy `try`, so the baseline is translated to exnref)
//!   2. `bynexec a.wasm` (wasmtime) -> expected output
//!   3. `wasm-opt a.wasm <features> <random pipeline>` -> b.wasm
//!      oracle crash: wasm-opt must not crash, assert, hang or fail validation
//!   4. `bynexec b.wasm` -> must equal the expected output (oracle exec)
//! Findings go to <out>/findings/<signature>/<n>/ with a self-contained repro.sh.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

mod interp;
mod mask64;
mod smith;
mod ss;
mod wat;

const OPT_TIMEOUT: u32 = 30;
const EXEC_TIMEOUT: u32 = 20;

/// Features wasmtime 50 cannot run (or that make results implementation
/// defined: relaxed SIMD may be folded by Precompute with other semantics).
const BASE_FEATURES: &[&str] = &[
    "-all",
    "--disable-strings",
    "--disable-fp16",
    "--disable-custom-descriptors",
    "--disable-multibyte",
    "--disable-compact-imports",
    "--disable-relaxed-atomics",
    "--disable-acquire-release-atomics",
    "--disable-shared-everything",
    "--disable-relaxed-simd",
];

/// Features randomly switched off per case, for variety.
const OPTIONAL_FEATURES: &[&str] = &[
    "simd",
    "stack-switching",
    "multimemory",
    "memory64",
    "tail-call",
    "threads",
    "wide-arithmetic",
    "custom-page-sizes",
    "exception-handling",
];

/// Pass groups, from scripts/fuzz_opt.py opt_choices (minus flatten-based ones,
/// which fuzz_opt also avoids with EH/multivalue+reftypes, and -O4 which
/// flattens), plus semantics-preserving passes fuzz_opt never selects.
const PASS_GROUPS: &[&[&str]] = &[
    &["-O1"], &["-O2"], &["-O3"], &["-Os"], &["-Oz"],
    &["--coalesce-locals"], &["--code-pushing"], &["--code-folding"], &["--const-hoisting"],
    &["--constraint-analysis"], &["--dae"], &["--dae-optimizing"], &["--dae2"], &["--dce"],
    &["--directize"], &["--discard-global-effects"], &["--duplicate-function-elimination"],
    &["--inlining"], &["--inlining-optimizing"], &["--generate-global-effects"],
    &["--global-refining"], &["--gsi"], &["--gufa"], &["--gufa-cast-all"], &["--gufa-optimizing"],
    &["--local-cse"], &["--heap2local"], &["--remove-unused-names", "--heap2local"],
    &["--heap-store-optimization"], &["--generate-stack-ir"], &["--licm"], &["--local-subtyping"],
    &["--mark-noreturn"], &["--memory-packing"], &["--merge-blocks"], &["--merge-locals"],
    &["--monomorphize", "--pass-arg=monomorphize-min-benefit@0"],
    &["--monomorphize", "--pass-arg=monomorphize-min-benefit@50"],
    &["--monomorphize-always"], &["--minimize-rec-groups"], &["--no-stack-ir"],
    &["--once-reduction"], &["--optimize-casts"], &["--optimize-instructions"],
    &["--optimize-noreturn"], &["--optimize-stack-ir"], &["--generate-stack-ir", "--optimize-stack-ir"],
    &["--generate-stack-ir", "--optimize-stack-ir", "--roundtrip"], &["--pick-load-signs"],
    &["--precompute"], &["--precompute-propagate"], &["--remove-unused-brs"],
    &["--remove-unused-nonfunction-module-elements"], &["--remove-unused-module-elements"],
    &["--remove-unused-names"], &["--reorder-functions"], &["--reorder-locals"], &["--roundtrip"],
    &["--rse"], &["--simplify-globals"], &["--simplify-globals-optimizing"], &["--simplify-locals"],
    &["--simplify-locals-nonesting"], &["--simplify-locals-nostructure"], &["--simplify-locals-notee"],
    &["--simplify-locals-notee-nostructure"], &["--ssa"], &["--strip-refining-casts"], &["--tail-call"],
    &["--tuple-optimization"], &["--unsubtyping"], &["--vacuum"],
    // not in fuzz_opt's opt_choices
    &["--merge-similar-functions"], &["--avoid-reinterprets"], &["--propagate-globals-globally"],
    &["--reorder-globals"], &["--untee"], &["--ssa-nomerge"], &["--duplicate-import-elimination"],
];

/// Features off in the "numeric" profile (some unseeded cases): flatten
/// cannot spill non-nullable refs/tuples or handle EH blocks, asyncify
/// supports neither references nor tail calls, outlining handles little more
/// than MVP.
const NUMERIC_OFF: &[&str] = &[
    "exception-handling", "gc", "reference-types", "multivalue", "tail-call", "stack-switching",
];

/// Which modules a lowering group applies to.
#[derive(Clone, Copy, PartialEq)]
enum Needs {
    Any,
    /// AlignmentLowering only handles scalar loads/stores ("unhandled unaligned load").
    NoSimd,
    /// The numeric profile (NUMERIC_OFF all disabled).
    Numeric,
    /// All memories alike: MultiMemoryLowering "assum[es] that each memory is
    /// configured the same as the first and assert[s]" (shared, address type,
    /// page size).
    UniformMemories,
}

/// Semantics-preserving lowering and restructuring passes that neither
/// fuzz_opt's opt_choices nor PASS_GROUPS run. One of them is inserted into
/// some pipelines (oracle exec/crash, signature names the pass).
const LOWER_GROUPS: &[(&[&str], Needs)] = &[
    // (--table64-lowering is an alias of it.)
    (&["--memory64-lowering"], Needs::Any),
    (&["--multi-memory-lowering-with-bounds-checks"], Needs::UniformMemories),
    (&["--signext-lowering"], Needs::Any),
    (&["--llvm-nontrapping-fptoint-lowering"], Needs::Any),
    (&["--dealign"], Needs::Any),
    (&["--optimize-for-js"], Needs::Any),
    (&["--coalesce-locals-learning"], Needs::Any),
    (&["--limit-segments"], Needs::Any),
    (&["--intrinsic-lowering"], Needs::Any),
    (&["--alignment-lowering"], Needs::NoSimd),
    (&["--dealign", "--alignment-lowering"], Needs::NoSimd),
    (&["--outlining"], Needs::Numeric),
    (&["--asyncify"], Needs::Numeric),
    (&["--flatten"], Needs::Numeric),
    (&["--flatten", "--dfo"], Needs::Numeric),
    (&["--flatten", "--rereloop"], Needs::Numeric),
    (&["--flatten", "--local-cse"], Needs::Numeric),
    (&["--flatten", "--simplify-locals-notee-nostructure", "--local-cse"], Needs::Numeric),
    (&["-O4"], Needs::Numeric),
];

/// Lowerings that by design change results the source program defines, so
/// they get the crash oracle only. llvm-nontrapping-fptoint-lowering: "if
/// fabs(operand) < INT_MAX then use the trapping operation, else return
/// INT_MIN ... otherwise it's UB so any value will do" (LLVM semantics).
const CRASH_ONLY_PASSES: &[&str] = &["--llvm-nontrapping-fptoint-lowering"];

/// Fatal errors by which a lowering pass states a documented limitation.
const LOWER_LIMITS: &[&str] = &[
    "MultiMemoryLowering: ",
    "not yet supported in asyncify",
    "Asyncify does not yet support",
    "Unsupported instruction for Flatten",
    // Asyncify needs --pass-arg=asyncify-memory@<name> with several memories.
    "Please specify which of the multiple memories to use",
];

fn lower_applies(needs: Needs, features: &[String]) -> bool {
    let off = |f: &str| features.iter().any(|x| x == &format!("--disable-{f}"));
    let numeric = NUMERIC_OFF.iter().all(|f| off(f));
    match needs {
        Needs::Any => true,
        Needs::NoSimd => off("simd"),
        Needs::UniformMemories => off("threads") && off("memory64") && off("custom-page-sizes"),
        Needs::Numeric => numeric,
    }
}

/// fuzz_opt.py requires_closed_world.
const CLOSED_WORLD_GROUPS: &[&[&str]] = &[
    &["--type-refining"], &["--type-refining-gufa"], &["--signature-pruning"], &["--signature-refining"],
    &["--gto"], &["--remove-unused-types"], &["--abstract-type-refining"], &["--cfp"], &["--cfp-reftest"],
    &["--reorder-types"], &["--type-finalizing"], &["--type-unfinalizing"], &["--type-ssa"],
    &["--type-merging"], &["--unsubtyping"],
];

pub struct Rng(u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

struct Ctx {
    bin: PathBuf,
    out: PathBuf,
    tmp: PathBuf,
    stats: BTreeMap<String, u64>,
    per_sig: BTreeMap<String, u64>,
    seed: u64,
    start: Instant,
    /// --initial-fuzz contents (build-dir/seeds, from the target's lit tests).
    seeds: Vec<PathBuf>,
    /// The current case contains an ss.rs stack-switching program.
    ss_case: bool,
    /// ... and it uses `switch`, which binaryen's interpreter cannot run.
    ss_switch: bool,
    /// a.wasm is a wasm-smith module as generated (no -ttf): bynexec also
    /// hashes exported memories (BYNEXEC_MEMHASH).
    smith_case: bool,
}

fn run_timeout(timeout: u32, prog: &Path, args: &[String], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new("timeout");
    c.arg("-k").arg("2").arg(timeout.to_string()).arg(prog).args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    c.stdin(Stdio::null()).output().expect("spawn timeout")
}

fn sanitize(s: &str) -> String {
    let t: String = s
        .trim_start_matches('-')
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    if t.is_empty() { "unknown".into() } else { t }
}

/// Reader limitations binaryen states in its error text (TODOs in
/// wasm-binary.cpp, MVP-only validation when reference types are off): not
/// reported by oracle binary-read.
const READ_LIMITS: &[&str] = &[
    "Recursion groups of size zero not supported",
    "should refer to a single table in MVP",
    "must be non-null funcrefs in MVP",
];

/// Stable sub-area of a binary-read failure, from the first error line:
/// the source file of an assertion, the type of an uncaught C++ exception,
/// else the first words of the parse
/// exception or validator message (numbers, names and offsets dropped).
fn read_area(kind: CrashKind, err: &str) -> String {
    let words = |m: &str| -> String {
        let w: Vec<String> = m
            .split(|c: char| !c.is_ascii_alphabetic())
            .filter(|w| w.len() > 1)
            .take(4)
            .map(|w| w.to_ascii_lowercase())
            .collect();
        if w.is_empty() { "unknown".into() } else { w.join("-") }
    };
    for l in err.lines() {
        if kind == CrashKind::Assert && (l.contains("Assertion") || l.contains("UNREACHABLE")) {
            if let Some(f) = l.split(|c: char| c.is_whitespace() || c == ':').find(|t| t.ends_with(".cpp") || t.ends_with(".h")) {
                let stem = f.rsplit('/').next().unwrap_or(f);
                return sanitize(stem.trim_end_matches(".cpp").trim_end_matches(".h"));
            }
        }
        // An uncaught C++ exception (e.g. std::length_error) names its type.
        if kind == CrashKind::Signal {
            if let Some(i) = l.find("throwing an instance of '") {
                let t = &l[i + 25..];
                let t = t.split('\'').next().unwrap_or(t);
                return sanitize(t.trim_start_matches("std::"));
            }
        }
        // Text parser errors: `Fatal: <file>:<line>:<col>: error: <message>`.
        if let Some(i) = l.find(": error: ") {
            return words(&l[i + 9..]);
        }
        if let Some(i) = l.find("[parse exception: ") {
            return words(&l[i + 18..]);
        }
        if l.contains("[wasm-validator error") {
            if let Some(i) = l.find("unexpected ") {
                let m = &l[i..];
                let m = m.split_once(": ").map(|x| x.1).unwrap_or(m);
                return words(m);
            }
        }
    }
    kind.name().into()
}

fn shell_quote(a: &str) -> String {
    if a.chars().all(|c| c.is_ascii_alphanumeric() || "-_=.@/,:".contains(c)) {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

fn shell_args(args: &[String]) -> String {
    args.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Copy, PartialEq)]
enum CrashKind {
    Assert,
    Signal,
    Hang,
    Validation,
    Fatal,
    Exit,
}

impl CrashKind {
    fn name(self) -> &'static str {
        match self {
            CrashKind::Assert => "assert",
            CrashKind::Signal => "signal",
            CrashKind::Hang => "hang",
            CrashKind::Validation => "validation",
            CrashKind::Fatal => "fatal",
            CrashKind::Exit => "exit",
        }
    }
}

fn crash_kind(o: &Output) -> Option<CrashKind> {
    use std::os::unix::process::ExitStatusExt;
    let err = String::from_utf8_lossy(&o.stderr);
    let code = o.status.code();
    if o.status.success() {
        return None;
    }
    if code == Some(124) || code == Some(137) {
        return Some(CrashKind::Hang);
    }
    if err.contains("Assertion") || err.contains("UNREACHABLE") {
        return Some(CrashKind::Assert);
    }
    if o.status.signal().is_some() || code.map_or(false, |c| c > 128) {
        return Some(CrashKind::Signal);
    }
    if err.contains("[wasm-validator error") || err.contains("error validating") {
        return Some(CrashKind::Validation);
    }
    if err.contains("Fatal") {
        return Some(CrashKind::Fatal);
    }
    Some(CrashKind::Exit)
}

/// Name of the pass wasm-opt was running when it failed (BINARYEN_PASS_DEBUG=1
/// also validates after every pass, so a validation failure names its pass).
fn failing_pass(ctx: &Ctx, args: &[String]) -> String {
    let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), args, &[("BINARYEN_PASS_DEBUG", "1")]);
    let err = String::from_utf8_lossy(&o.stderr);
    let mut last = None;
    for l in err.lines() {
        if let Some(i) = l.find("running pass: ") {
            let rest = &l[i + "running pass: ".len()..];
            let name = rest.split("...").next().unwrap_or(rest).trim();
            last = Some(name.to_string());
        }
    }
    match last {
        Some(p) => sanitize(&p),
        None => "io".into(),
    }
}

impl Ctx {
    fn bump(&mut self, k: &str) {
        *self.stats.entry(k.to_string()).or_insert(0) += 1;
    }

    fn write_stats(&self) {
        let mut s = String::from("{");
        let mut first = true;
        for (k, v) in &self.stats {
            if !first {
                s.push_str(", ");
            }
            first = false;
            s.push_str(&format!("\"{k}\": {v}"));
        }
        let secs = self.start.elapsed().as_secs_f64().max(0.001);
        let cps = *self.stats.get("cases").unwrap_or(&0) as f64 / secs;
        s.push_str(&format!(", \"cases_per_sec\": {cps:.2}}}\n"));
        let tmp = self.out.join("stats.json.tmp");
        let _ = fs::write(&tmp, s);
        let _ = fs::rename(&tmp, self.out.join("stats.json"));
    }

    /// Save a finding (at most 3 per signature). `files` are copied into the
    /// case directory; `repro` is the body of repro.sh.
    fn finding(&mut self, sig: &str, oracle: &str, message: &str, files: &[(&str, &Path)], repro: &str, extra: &[(&str, String)]) {
        self.bump("failures");
        self.bump(&format!("fail:{sig}"));
        let n = self.per_sig.entry(sig.to_string()).or_insert(0);
        *n += 1;
        if *n > 3 {
            return;
        }
        let dir = self.out.join("findings").join(sig).join(n.to_string());
        let _ = fs::create_dir_all(&dir);
        for (name, src) in files {
            let _ = fs::copy(src, dir.join(name));
        }
        for (name, content) in extra {
            let _ = fs::write(dir.join(name), content);
        }
        let msg_json = message.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', " ");
        let msg_json: String = msg_json.chars().filter(|c| !c.is_control()).collect();
        let fj = format!(
            "{{\"signature\": \"{sig}\", \"oracle\": \"{oracle}\", \"message\": \"{msg_json}\", \"seed\": {}}}\n",
            self.seed
        );
        let _ = fs::write(dir.join("finding.json"), fj);
        let rp = dir.join("repro.sh");
        let repro = if self.smith_case && repro.contains("/bin/bynexec\"") {
            repro.replacen("trap 'rm -rf \"$T\"' EXIT\n", "trap 'rm -rf \"$T\"' EXIT\nexport BYNEXEC_MEMHASH=1\n", 1)
        } else {
            repro.to_string()
        };
        let _ = fs::write(&rp, repro);
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&rp, fs::Permissions::from_mode(0o755));
        eprintln!("[bynfuzz] finding {sig}: {}", message.lines().next().unwrap_or(""));
    }
}

const REPRO_HEAD: &str = r#"#!/bin/bash
# Usage: repro.sh <build-dir>   (exit 1: reproduces, 0: does not, 2: undecided)
B=$(realpath "$1")
cd "$(dirname "$0")"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
"#;

fn crash_repro(input: &str, args_after_input: &[String], kind: CrashKind, err: &str) -> String {
    // For asserts, require the same assertion text.
    let assert_line = err
        .lines()
        .find(|l| l.contains("Assertion") || l.contains("UNREACHABLE"))
        .map(|l| l.split(" at ").next().unwrap_or(l).to_string())
        .unwrap_or_default();
    let check = match kind {
        CrashKind::Hang => "[ $rc = 124 ] || [ $rc = 137 ] && exit 1".to_string(),
        CrashKind::Assert => format!("grep -qF -- {} \"$T/err\" && exit 1", shell_quote(assert_line.trim())),
        CrashKind::Signal => "[ $rc -gt 128 ] && [ $rc != 137 ] && exit 1".to_string(),
        CrashKind::Validation => "grep -q -e 'wasm-validator error' -e 'error validating' \"$T/err\" && exit 1".to_string(),
        CrashKind::Fatal => "grep -q Fatal \"$T/err\" && exit 1".to_string(),
        CrashKind::Exit => "exit 1".to_string(),
    };
    format!(
        "{REPRO_HEAD}timeout -k 2 {OPT_TIMEOUT} \"$B/bin/wasm-opt\" {input} {} -o \"$T/out.wasm\" > \"$T/err\" 2>&1\nrc=$?\n[ $rc = 0 ] && exit 0\n{check}\nexit 2\n",
        shell_args(args_after_input)
    )
}

fn exec_repro(features: &[String], pipeline: &[String], what: &str) -> String {
    let opt = format!(
        "timeout -k 2 {OPT_TIMEOUT} \"$B/bin/wasm-opt\" a.wasm {} {} -o \"$T/b.wasm\" 2>/dev/null || exit 2\n",
        shell_args(features),
        shell_args(pipeline)
    );
    let exec = format!(
        "timeout -k 2 {EXEC_TIMEOUT} \"$B/bin/bynexec\" a.wasm > \"$T/a.out\" 2>/dev/null; ra=$?\n[ $ra = 0 ] || exit 2\ntimeout -k 2 {EXEC_TIMEOUT} \"$B/bin/bynexec\" \"$T/b.wasm\" > \"$T/b.out\" 2>\"$T/b.err\"; rb=$?\n"
    );
    let check = match what {
        "mismatch" => "[ $rb = 0 ] || exit 2\ncmp -s \"$T/a.out\" \"$T/b.out\" && exit 0\ndiff \"$T/a.out\" \"$T/b.out\" | head -20\nexit 1\n",
        "reject" => "[ $rb = 4 ] && { head -5 \"$T/b.err\"; exit 1; }\n[ $rb = 0 ] && exit 0\nexit 2\n",
        _ => "[ $rb = 0 ] && exit 0\n[ $rb = 3 ] && exit 2\n[ $rb = 4 ] && exit 2\n[ $rb = 124 ] && exit 2\nexit 1\n",
    };
    format!("{REPRO_HEAD}{opt}{exec}{check}")
}

enum Exec {
    Ok(Vec<u8>),
    Limit,
    Reject(String),
    Timeout,
    Crash(String),
}

fn exec(ctx: &Ctx, wasm: &Path) -> Exec {
    let mut env = vec![("BYNEXEC_SPLIT_I64", "1")];
    if ctx.smith_case {
        env.push(("BYNEXEC_MEMHASH", "1"));
    }
    let o = run_timeout(EXEC_TIMEOUT, &ctx.bin.join("bynexec"), &[wasm.display().to_string()], &env);
    let err = String::from_utf8_lossy(&o.stderr).to_string();
    match o.status.code() {
        Some(0) => Exec::Ok(o.stdout),
        Some(3) => Exec::Limit,
        Some(4) => Exec::Reject(err),
        Some(124) | Some(137) => Exec::Timeout,
        _ => Exec::Crash(err),
    }
}

fn first_diff(a: &[u8], b: &[u8]) -> String {
    let a = String::from_utf8_lossy(a);
    let b = String::from_utf8_lossy(b);
    let al: Vec<&str> = a.lines().collect();
    let bl: Vec<&str> = b.lines().collect();
    for i in 0..al.len().max(bl.len()) {
        let x = al.get(i).copied().unwrap_or("<eof>");
        let y = bl.get(i).copied().unwrap_or("<eof>");
        if x != y {
            return format!("line {}: before `{x}` after `{y}`", i + 1);
        }
    }
    "outputs differ".into()
}

/// Is b (a.wasm optimized by `pipeline`) still different from `expected`?
/// The first line that differs is a trap in the expected output.
fn first_diff_is_trap(a: &[u8], b: &[u8]) -> bool {
    let (a, b) = (String::from_utf8_lossy(a), String::from_utf8_lossy(b));
    let mut bl = b.lines();
    for la in a.lines() {
        if bl.next() != Some(la) {
            // `[trap]`, `[instantiate: trap]`, ...
            return la.contains("trap]");
        }
    }
    false
}

/// Memory64Lowering turns a 64-bit memory into a 32-bit one ("lower loads and
/// stores to a 64-bit memory to instead use a 32-bit one", pass.cpp) by
/// wrapping every address to i32; a module that uses an address of 4 GiB or
/// more "cannot be correctly lowered" (commit 18ba0616, #8316), so an
/// out-of-bounds access through such a pointer no longer traps by design.
/// True when the first lost trap is, per binaryen's interpreter on a.wasm, a
/// load/store whose pointer is >= 2^32 (`final > memory: <ptr> > ...`), or
/// whose static offset is >= 2^32 (`offset > memory: <offset> > ...`): the
/// lowering itself keeps such a trap (offsets >= 4G become unreachable,
/// #8316), but an earlier pass may exactly fold the offset into a constant
/// pointer (OptimizeInstructions: `offset=2^33 (i64.const 2^32)` becomes
/// `(i64.const 3*2^32)`), which the lowering then wraps into bounds.
fn wrapped_address_trap(ctx: &Ctx, a: &Path, features: &[String], expected: &[u8], out: &[u8]) -> bool {
    let (ea, eb) = (String::from_utf8_lossy(expected), String::from_utf8_lossy(out));
    let mut bl = eb.lines();
    let mut export = None;
    for la in ea.lines() {
        if bl.next() != Some(la) {
            break;
        }
        if let Some(e) = la.strip_prefix("[fuzz-exec] export ") {
            export = Some(e.to_string());
        }
    }
    let Some(export) = export else { return false };
    let Some(io) = interp_run(&ctx.bin, a, features) else { return false };
    let header = format!("[fuzz-exec] export {export}");
    let mut lines = io.lines().skip_while(|l| *l != header).skip(1);
    let Some(t) = lines.find(|l| l.starts_with("[trap") || l.starts_with("[fuzz-exec] export ")) else { return false };
    ["[trap final > memory: ", "[trap highest > memory: ", "[trap offset > memory: "].iter().any(|p| {
        t.strip_prefix(p)
            .and_then(|r| r.split(' ').next())
            .and_then(|n| n.parse::<u64>().ok())
            .map_or(false, |n| n >= 1u64 << 32)
    })
}

/// Memory64Lowering wraps every 64-bit address, size and delta operand to i32
/// (wrapAddress64); a module that needs values of 2^32 or more "cannot be
/// correctly lowered" (commit 18ba0616, #8316). E.g. a table.grow by 2^32+1
/// fails (-1) on the i64 table but grows by 1 after lowering. True when a.wasm
/// with each such operand masked to its low 32 bits (still a 64-bit module)
/// gives exactly the lowered output `out`: the difference came only from
/// operands >= 2^32, and any other change by the lowering stays reported.
fn masked_matches(ctx: &Ctx, a: &Path, features: &[String], out: &[u8]) -> bool {
    // Print with the case's features, not wasm-dis: without GC enabled the
    // printer leaves out `(type $t)` of functions whose type has a declared
    // supertype, so the text would not reparse (ref.func gets an MVP type).
    let (src, wat, m) = (ctx.tmp.join("unmasked.wat"), ctx.tmp.join("masked.wat"), ctx.tmp.join("masked.wasm"));
    let mut args = vec![a.display().to_string()];
    args.extend(features.iter().cloned());
    args.extend(["-S".to_string(), "-o".to_string(), src.display().to_string()]);
    if !run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[]).status.success() {
        return false;
    }
    let Ok(text) = fs::read_to_string(&src) else { return false };
    let Some(masked) = mask64::mask(&text) else { return false };
    if fs::write(&wat, masked).is_err() {
        return false;
    }
    let mut args = vec![wat.display().to_string()];
    args.extend(features.iter().cloned());
    args.extend(["-o".to_string(), m.display().to_string()]);
    if !run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[]).status.success() {
        return false;
    }
    matches!(exec(ctx, &m), Exec::Ok(mo) if mo == out)
}

fn mismatches(ctx: &Ctx,a: &Path, features: &[String], pipeline: &[String], expected: &[u8]) -> bool {
    let b = ctx.tmp.join("bisect.wasm");
    let mut args = vec![a.display().to_string()];
    args.extend(features.iter().cloned());
    args.extend(pipeline.iter().cloned());
    args.push("-o".into());
    args.push(b.display().to_string());
    let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[]);
    if !o.status.success() {
        return false;
    }
    matches!(exec(ctx, &b), Exec::Ok(out) if out != expected)
}

/// Localize a mismatch to one pass: expand the pipeline to the passes it ran
/// (BINARYEN_PASS_DEBUG=1 prints them), check the explicit list still
/// mismatches, then binary-search the shortest failing prefix.
fn blame_pass(ctx: &Ctx, a: &Path, features: &[String], pipeline: &[String], expected: &[u8]) -> Option<String> {
    let mut args = vec![a.display().to_string()];
    args.extend(features.iter().cloned());
    args.extend(pipeline.iter().cloned());
    args.push("-o".into());
    args.push("/dev/null".into());
    let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[("BINARYEN_PASS_DEBUG", "1")]);
    let err = String::from_utf8_lossy(&o.stderr);
    let mut passes = Vec::new();
    for l in err.lines() {
        // Top-level passes only: nested runners indent further.
        if let Some(rest) = l.strip_prefix("[PassRunner]   running pass: ") {
            let name = rest.split("...").next().unwrap_or(rest).trim();
            passes.push(format!("--{name}"));
        }
    }
    if passes.is_empty() {
        return None;
    }
    // Keep the global flags (levels, closed world, pass args, inlining knobs):
    // everything that is neither -O nor a pass name.
    let is_pass = |f: &str| {
        (PASS_GROUPS.iter().chain(CLOSED_WORLD_GROUPS.iter()).any(|g| g.contains(&f))
            || LOWER_GROUPS.iter().any(|(g, _)| g.contains(&f)))
            && !f.starts_with("--pass-arg")
    };
    let flags: Vec<String> =
        pipeline.iter().filter(|f| !f.starts_with("-O") && !is_pass(f)).cloned().collect();
    let mut levels = Vec::new();
    for f in pipeline {
        let lv = match f.as_str() {
            "-O1" => Some(("1", "0")),
            "-O2" => Some(("2", "0")),
            "-O3" => Some(("3", "0")),
            "-O4" => Some(("4", "0")),
            "-Os" => Some(("2", "1")),
            "-Oz" => Some(("2", "2")),
            _ => None,
        };
        if let Some((o, s)) = lv {
            levels = vec![format!("--optimize-level={o}"), format!("--shrink-level={s}")];
        }
    }
    let with = |k: usize| -> Vec<String> {
        let mut v = levels.clone();
        v.extend(flags.iter().cloned());
        v.extend(passes[..k].iter().cloned());
        v
    };
    if !mismatches(ctx, a, features, &with(passes.len()), expected) {
        return None;
    }
    let (mut lo, mut hi) = (0usize, passes.len()); // prefix lo passes ok (assumed), hi fails
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if mismatches(ctx, a, features, &with(mid), expected) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(sanitize(&passes[hi - 1]))
}

fn random_features(rng: &mut Rng) -> Vec<String> {
    let mut f: Vec<String> = BASE_FEATURES.iter().map(|s| s.to_string()).collect();
    let numeric = rng.chance(0.15);
    for opt in OPTIONAL_FEATURES {
        if rng.chance(0.15) || (numeric && NUMERIC_OFF.contains(opt)) {
            f.push(format!("--disable-{opt}"));
        }
    }
    if numeric {
        for opt in NUMERIC_OFF {
            if !OPTIONAL_FEATURES.contains(opt) {
                f.push(format!("--disable-{opt}"));
            }
        }
        if rng.chance(0.5) {
            for opt in ["memory64", "threads", "custom-page-sizes"] {
                f.push(format!("--disable-{opt}"));
            }
        }
    }
    f
}

fn random_pipeline(rng: &mut Rng, closed_world: bool, features: &[String]) -> Vec<String> {
    let mut groups: Vec<&[&str]> = Vec::new();
    loop {
        let g = if closed_world && rng.chance(0.25) { *rng.pick(CLOSED_WORLD_GROUPS) } else { *rng.pick(PASS_GROUPS) };
        groups.push(g);
        if groups.len() > 20 || rng.chance(0.3) {
            break;
        }
    }
    let numeric = lower_applies(Needs::Numeric, features);
    if rng.chance(if numeric { 0.6 } else { 0.3 }) {
        let usable: Vec<&[&str]> =
            LOWER_GROUPS.iter().filter(|(_, n)| lower_applies(*n, features)).map(|(g, _)| *g).collect();
        let g = *rng.pick(&usable);
        // Lowering passes run last in real toolchains; elsewhere they meet
        // optimized-then-lowered and lowered-then-optimized code.
        let pos = if rng.chance(0.5) { groups.len() } else { rng.below(groups.len() + 1) };
        groups.insert(pos, g);
        if rng.chance(0.2) {
            groups.truncate(pos + 1);
            groups.drain(..pos);
        }
    }
    if rng.chance(0.3) {
        let pos = rng.below(groups.len() + 1);
        groups.insert(pos, &["--roundtrip"]);
    }
    let mut ret: Vec<String> = groups.iter().flat_map(|g| g.iter().map(|s| s.to_string())).collect();
    if !ret.iter().any(|f| f.starts_with("-O")) {
        if rng.chance(0.5) {
            ret.push(format!("--optimize-level={}", rng.below(4)));
        }
        if rng.chance(0.5) {
            ret.push(format!("--shrink-level={}", rng.below(3)));
        }
    }
    // --converge reruns the whole pipeline: lowerings are one-shot (asyncify
    // adds its globals again), and fuzz_opt avoids repeated flattening
    // ("exponential overhead").
    let one_shot = ret.iter().any(|f| f == "-O4" || LOWER_GROUPS.iter().any(|(g, _)| g.contains(&f.as_str())));
    if rng.chance(0.05) && !one_shot {
        ret.push("--converge".into());
    }
    if rng.chance(0.5) {
        ret.push("-fimfs=99999999".into());
    }
    if rng.chance(0.5) {
        ret.push("-pii=4".into());
    }
    if rng.chance(0.5) {
        ret.push("--expect-later-toolchain-opts".into());
    }
    if closed_world {
        ret.push("--closed-world".into());
    }
    // Self-test only: append flags (e.g. -tnh) to check the oracle and repro path.
    if let Ok(extra) = std::env::var("BYNFUZZ_EXTRA_OPTS") {
        ret.extend(extra.split_whitespace().map(|s| s.to_string()));
    }
    ret
}

/// Binaryen's interpreter on `wasm` (what `--fuzz-exec` records before
/// optimizing). None when it did not finish (timeout, crash).
fn interp_run(bin: &Path, wasm: &Path, features: &[String]) -> Option<String> {
    let mut args = vec![wasm.display().to_string()];
    args.extend(features.iter().cloned());
    args.push("--fuzz-exec-before".into());
    let o = run_timeout(EXEC_TIMEOUT, &bin.join("wasm-opt"), &args, &[]);
    if !o.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&o.stdout).to_string())
}

fn interp_repro(wasm: &str, features: &[String]) -> String {
    format!(
        "{REPRO_HEAD}\"$B/bin/bynfuzz\" --interp-check \"$B\" {wasm} {}\n",
        shell_args(features)
    )
}

/// Oracle interp on one module whose wasmtime output is `wt`.
/// `which` is "a" (generated) or "b" (optimized) for the counters.
fn interp_oracle(ctx: &mut Ctx, wasm: &Path, features: &[String], wt: &[u8], which: &str, cmdline: &str) {
    if ctx.ss_switch {
        return ctx.bump("interp_skip_switch");
    }
    let Some(bo) = interp_run(&ctx.bin, wasm, features) else {
        return ctx.bump("interp_skip_failed");
    };
    let wts: String = String::from_utf8_lossy(wt).lines().filter(|l| !l.starts_with("[memhash ")).map(|l| format!("{l}\n")).collect();
    match interp::compare(&bo, &wts) {
        interp::Verdict::Same => ctx.bump("interp_same"),
        interp::Verdict::Skip(k) => ctx.bump(k),
        interp::Verdict::Differ(class, msg) => {
            let sig = if ctx.ss_case {
                format!("interp__mismatch__ss-{class}")
            } else if ctx.smith_case {
                format!("interp__mismatch__smith-{class}")
            } else {
                format!("interp__mismatch__{class}")
            };
            let msg = format!("binaryen interpreter vs wasmtime on the {which} module: {msg}");
            let repro = interp_repro("m.wasm", features);
            ctx.finding(&sig, "interp", &msg, &[("m.wasm", wasm)], &repro,
                &[("binaryen.out", bo), ("wasmtime.out", wts), ("cmd.txt", cmdline.to_string() + "\n")]);
        }
    }
}

/// `wasm-opt m.wasm <features> --translate-to-exnref -o out` (for repro.sh).
fn translate_exnref(bin: &Path, m: &Path, features: &[String], out: &Path) -> bool {
    let mut args = vec![m.display().to_string()];
    args.extend(features.iter().cloned());
    args.extend(["--translate-to-exnref".to_string(), "-o".to_string(), out.display().to_string()]);
    run_timeout(OPT_TIMEOUT, &bin.join("wasm-opt"), &args, &[]).status.success()
}

/// `bynfuzz --interp-check <build> <m.wasm> [--translate] <features...>`: the
/// interp oracle for repro.sh. With --translate, wasmtime runs m.wasm after
/// --translate-to-exnref (binaryen's interpreter still runs m.wasm itself).
/// Exit 1: outputs differ, 0: same, 2: undecided.
fn interp_check(args: &[String]) -> i32 {
    let bin = PathBuf::from(&args[0]).join("bin");
    let wasm = PathBuf::from(&args[1]);
    let translate = args.get(2).map_or(false, |a| a == "--translate");
    let features = &args[if translate { 3 } else { 2 }..];
    let mut run_wasm = wasm.clone();
    let tmp = std::env::temp_dir().join(format!("interp-check-{}.wasm", std::process::id()));
    if translate {
        if !translate_exnref(&bin, &wasm, features, &tmp) {
            return 2;
        }
        run_wasm = tmp.clone();
    }
    let o = run_timeout(EXEC_TIMEOUT, &bin.join("bynexec"), &[run_wasm.display().to_string()], &[("BYNEXEC_SPLIT_I64", "1")]);
    let _ = fs::remove_file(&tmp);
    if o.status.code() != Some(0) {
        return 2;
    }
    let wt = String::from_utf8_lossy(&o.stdout).to_string();
    let Some(bo) = interp_run(&bin, &wasm, features) else { return 2 };
    match interp::compare(&bo, &wt) {
        interp::Verdict::Same => 0,
        interp::Verdict::Skip(_) => 2,
        interp::Verdict::Differ(_, msg) => {
            println!("{msg}");
            1
        }
    }
}

/// Oracle interp on the module as -ttf made it, before --translate-to-exnref
/// (legacy try/catch/delegate/rethrow, which wasmtime cannot run): binaryen's
/// interpreter on it must match wasmtime on its translation. A difference is
/// a bug in the translation or in the interpreter's legacy EH.
fn pre_exnref_oracle(ctx: &mut Ctx, gen_args: &[String], features: &[String], a: &Path, expected: &[u8]) {
    let a0 = ctx.tmp.join("a0.wasm");
    let a1 = ctx.tmp.join("a1.wasm");
    let mut args: Vec<String> = gen_args.iter().filter(|x| *x != "--translate-to-exnref").cloned().collect();
    args.extend(["-o".to_string(), a0.display().to_string()]);
    if !run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[]).status.success() {
        return ctx.bump("pre_exnref_skip_gen");
    }
    // The same translation repro.sh does (normally identical to a.wasm).
    // Unlike the in-process one at generation, this runs on the module as
    // read back from its binary, so it is a crash oracle of its own.
    let mut targs = vec![a0.display().to_string()];
    targs.extend(features.iter().cloned());
    targs.push("--translate-to-exnref".into());
    let mut full = targs.clone();
    full.extend(["-o".to_string(), a1.display().to_string()]);
    let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &full, &[]);
    if let Some(kind) = crash_kind(&o) {
        let err = String::from_utf8_lossy(&o.stderr).to_string();
        let sig = format!("crash__{}__translate-to-exnref-reload", kind.name());
        let repro = crash_repro("a0.wasm", &targs[1..], kind, &err);
        let msg = format!("--translate-to-exnref on a valid legacy-EH binary failed ({}): {}", kind.name(),
            err.lines().filter(|l| l.contains("error") || l.contains("Fatal") || l.contains("Assert")).take(3).collect::<Vec<_>>().join(" | "));
        let cmd = format!("wasm-opt a0.wasm {}\n", shell_args(&targs[1..]));
        ctx.finding(&sig, "crash", &msg, &[("a0.wasm", &a0)], &repro, &[("stderr.txt", err), ("cmd.txt", cmd)]);
        return;
    }
    let wt = if fs::read(&a1).ok() == fs::read(a).ok() {
        expected.to_vec()
    } else {
        match exec(ctx, &a1) {
            Exec::Ok(o) => o,
            _ => return ctx.bump("pre_exnref_skip_exec"),
        }
    };
    let Some(bo) = interp_run(&ctx.bin, &a0, features) else {
        return ctx.bump("pre_exnref_skip_interp");
    };
    let wts = String::from_utf8_lossy(&wt).to_string();
    match interp::compare(&bo, &wts) {
        interp::Verdict::Same => ctx.bump("pre_exnref_same"),
        interp::Verdict::Skip(k) => ctx.bump(k),
        interp::Verdict::Differ(class, msg) => {
            let sig = format!("interp__mismatch__pre-exnref-{class}");
            let msg = format!("binaryen interpreter on the legacy-EH module vs wasmtime on its --translate-to-exnref output: {msg}");
            let f = shell_args(features);
            let repro = format!("{REPRO_HEAD}\"$B/bin/bynfuzz\" --interp-check \"$B\" a0.wasm --translate {f}\n");
            ctx.finding(&sig, "interp", &msg, &[("a0.wasm", &a0)], &repro,
                &[("binaryen.out", bo), ("wasmtime.out", wts)]);
        }
    }
}

/// Oracle roundtrip: print b.wasm as text and parse it back; the module must
/// survive (no crash, valid) and, for de-NaN'd modules, run the same.
fn text_roundtrip(ctx: &mut Ctx, b: &Path, features: &[String], b_out: Option<&[u8]>, cmdline: &str) {
    ctx.bump("roundtrip");
    let wat = ctx.tmp.join("b.wat");
    let c = ctx.tmp.join("c.wasm");
    let _ = fs::remove_file(&wat);
    let _ = fs::remove_file(&c);
    // --name-types as in fuzz_opt.py RoundtripText: default type names may be
    // truncated and collide (a known FIXME there).
    let mut tail: Vec<String> = features.to_vec();
    tail.extend(["--name-types".to_string(), "-S".to_string()]);
    let mut args = vec![b.display().to_string()];
    args.extend(tail.iter().cloned());
    args.extend(["-o".to_string(), wat.display().to_string()]);
    let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[]);
    if let Some(kind) = crash_kind(&o) {
        let err = String::from_utf8_lossy(&o.stderr).to_string();
        let sig = format!("crash__{}__text-print", kind.name());
        let repro = crash_repro("b.wasm", &tail, kind, &err);
        let msg = format!("printing the optimized module as text failed ({}): {}", kind.name(), err.lines().rev().take(3).collect::<Vec<_>>().join(" | "));
        ctx.finding(&sig, "roundtrip", &msg, &[("b.wasm", b)], &repro, &[("stderr.txt", err), ("cmd.txt", cmdline.to_string() + "\n")]);
        return;
    }
    let mut args = vec![wat.display().to_string()];
    args.extend(features.iter().cloned());
    args.extend(["-o".to_string(), c.display().to_string()]);
    let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[]);
    if let Some(kind) = crash_kind(&o) {
        let err = String::from_utf8_lossy(&o.stderr).to_string();
        let sig = format!("crash__{}__text-parse", kind.name());
        let repro = crash_repro("b.wat", features, kind, &err);
        let msg = format!("parsing the printed module failed ({}): {}", kind.name(), err.lines().rev().take(3).collect::<Vec<_>>().join(" | "));
        ctx.finding(&sig, "roundtrip", &msg, &[("b.wasm", b), ("b.wat", &wat)], &repro, &[("stderr.txt", err), ("cmd.txt", cmdline.to_string() + "\n")]);
        return;
    }
    let Some(expected) = b_out else { return };
    match exec(ctx, &c) {
        Exec::Ok(out) if out == expected => ctx.bump("roundtrip_same"),
        Exec::Ok(out) => {
            let (ea, eb) = (String::from_utf8_lossy(expected).to_string(), String::from_utf8_lossy(&out).to_string());
            let class = match interp::compare(&ea, &eb) {
                interp::Verdict::Differ(c, _) => c,
                _ => "other",
            };
            let sig = format!("roundtrip__mismatch__{class}");
            let msg = format!("text round trip changed wasmtime output: {}", first_diff(expected, &out));
            let f = shell_args(features);
            let repro = format!(
                "{REPRO_HEAD}timeout -k 2 {OPT_TIMEOUT} \"$B/bin/wasm-opt\" b.wasm {f} --name-types -S -o \"$T/b.wat\" 2>/dev/null || exit 2\n\
                 timeout -k 2 {OPT_TIMEOUT} \"$B/bin/wasm-opt\" \"$T/b.wat\" {f} -o \"$T/c.wasm\" 2>/dev/null || exit 2\n\
                 timeout -k 2 {EXEC_TIMEOUT} \"$B/bin/bynexec\" b.wasm > \"$T/b.out\" 2>/dev/null || exit 2\n\
                 timeout -k 2 {EXEC_TIMEOUT} \"$B/bin/bynexec\" \"$T/c.wasm\" > \"$T/c.out\" 2>/dev/null || exit 2\n\
                 cmp -s \"$T/b.out\" \"$T/c.out\" && exit 0\ndiff \"$T/b.out\" \"$T/c.out\" | head -20\nexit 1\n"
            );
            ctx.finding(&sig, "roundtrip", &msg, &[("b.wasm", b), ("b.wat", &wat), ("c.wasm", &c)], &repro,
                &[("b.out", ea), ("c.out", eb), ("cmd.txt", cmdline.to_string() + "\n")]);
        }
        Exec::Reject(err) => {
            let msg = format!("wasmtime rejects the re-parsed text module: {}", err.lines().take(4).collect::<Vec<_>>().join(" | "));
            let f = shell_args(features);
            let repro = format!(
                "{REPRO_HEAD}timeout -k 2 {OPT_TIMEOUT} \"$B/bin/wasm-opt\" b.wasm {f} --name-types -S -o \"$T/b.wat\" 2>/dev/null || exit 2\n\
                 timeout -k 2 {OPT_TIMEOUT} \"$B/bin/wasm-opt\" \"$T/b.wat\" {f} -o \"$T/c.wasm\" 2>/dev/null || exit 2\n\
                 timeout -k 2 {EXEC_TIMEOUT} \"$B/bin/bynexec\" b.wasm > /dev/null 2>&1 || exit 2\n\
                 timeout -k 2 {EXEC_TIMEOUT} \"$B/bin/bynexec\" \"$T/c.wasm\" > /dev/null 2>&1; rc=$?\n[ $rc = 4 ] && exit 1\n[ $rc = 0 ] && exit 0\nexit 2\n"
            );
            ctx.finding("roundtrip__reject__text", "roundtrip", &msg, &[("b.wasm", b), ("b.wat", &wat)], &repro, &[("cmd.txt", cmdline.to_string() + "\n")]);
        }
        _ => ctx.bump("roundtrip_skip_exec"),
    }
}

/// Strip flags for oracle text-read: the comparison is on the module, not on
/// names or custom sections (binaryen does not keep `@custom` annotations).
const TEXT_READ_STRIP: &[&str] = &["--strip-debug", "--strip-producers", "--strip-target-features"];

/// wasm-opt -S of `m` without custom-section comment lines.
fn print_canonical(ctx: &Ctx, m: &Path, features: &[String]) -> Option<String> {
    let mut args = vec![m.display().to_string()];
    args.extend(features.iter().cloned());
    args.push("-S".into());
    let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[]);
    if !o.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.trim_start().starts_with(";; custom section")).collect::<Vec<_>>().join("\n"))
}

/// Stable name for the first differing line of two printed modules: its
/// first keyword (e.g. `i32.load`, `type`, `elem`), numbers and names dropped.
fn text_diff_area(a: &str, b: &str) -> String {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let i = (0..la.len().max(lb.len())).find(|&i| la.get(i) != lb.get(i)).unwrap_or(0);
    let line = la.get(i).or(lb.get(i)).copied().unwrap_or("");
    let kw = line
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .find(|t| t.chars().next().map_or(false, |c| c.is_ascii_alphabetic()))
        .unwrap_or("unknown");
    sanitize(&kw.replace('.', "-"))
}

/// Oracle text-read: `m` printed as standard text by a second implementation
/// (wasmprinter, folded or flat, with or without synthesized names) must parse
/// in wasm-opt's text parser and give the same module as reading the binary.
/// `expected` is the wasmtime output of `m` (de-NaN'd modules only).
fn text_read(ctx: &mut Ctx, rng: &mut Rng, m: &Path, features: &[String], expected: Option<&[u8]>, which: &str) {
    let Ok(bytes) = fs::read(m) else { return };
    let mut fold = rng.chance(0.5);
    // Binaryen's text parser takes a table initializer only when it starts
    // with `(` (an open bug: flat `ref.null func` there is "expected end of
    // table declaration"); 98% of such modules are printed folded so the bug
    // does not drown everything else.
    if !fold && smith::has_table_init(&bytes) && rng.chance(0.98) {
        ctx.bump("text_read_throttled_table_init");
        fold = true;
    }
    let names = rng.chance(0.3);
    let Some(text) = wat::print(&bytes, fold, names) else {
        return ctx.bump("text_read_unprintable");
    };
    ctx.bump("text_read");
    let mwasm = ctx.tmp.join("m.wasm");
    let mwat = ctx.tmp.join("m.wat");
    let r1 = ctx.tmp.join("r1.wasm");
    let r2 = ctx.tmp.join("r2.wasm");
    let _ = fs::remove_file(&r1);
    let _ = fs::remove_file(&r2);
    fs::write(&mwasm, &bytes).unwrap();
    fs::write(&mwat, &text).unwrap();
    let mut tail: Vec<String> = features.to_vec();
    tail.extend(TEXT_READ_STRIP.iter().map(|s| s.to_string()));
    let cmd = format!("wasmprinter (fold_instructions={fold}, name_unnamed={names}) {which}.wasm > m.wat\nwasm-opt m.wat {}\n", shell_args(&tail));
    let mut args = vec![mwasm.display().to_string()];
    args.extend(tail.iter().cloned());
    args.extend(["-o".to_string(), r1.display().to_string()]);
    let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[]);
    if !o.status.success() {
        return ctx.bump("text_read_skip_binary");
    }
    let mut args = vec![mwat.display().to_string()];
    args.extend(tail.iter().cloned());
    args.extend(["-o".to_string(), r2.display().to_string()]);
    let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[]);
    if let Some(kind) = crash_kind(&o) {
        let err = String::from_utf8_lossy(&o.stderr).to_string();
        let sig = format!("crash__{}__text-read-{}", kind.name(), read_area(kind, &err));
        let repro = crash_repro("m.wat", &tail, kind, &err);
        let msg = format!("wasm-opt fails to read the standard text (wasmprinter) of a module it reads as binary ({}): {}", kind.name(), err.lines().rev().take(3).collect::<Vec<_>>().join(" | "));
        ctx.finding(&sig, "text-read", &msg, &[("m.wasm", &mwasm), ("m.wat", &mwat)], &repro, &[("stderr.txt", err), ("cmd.txt", cmd)]);
        return;
    }
    if fs::read(&r1).ok() == fs::read(&r2).ok() {
        return ctx.bump("text_read_same");
    }
    let (Some(p1), Some(p2)) = (print_canonical(ctx, &r1, features), print_canonical(ctx, &r2, features)) else {
        return ctx.bump("text_read_skip_print");
    };
    if p1 == p2 {
        return ctx.bump("text_read_same_text");
    }
    let f = shell_args(features);
    let s = shell_args(&tail);
    let head = format!(
        "{REPRO_HEAD}timeout -k 2 {OPT_TIMEOUT} \"$B/bin/wasm-opt\" m.wasm {s} -o \"$T/r1.wasm\" 2>/dev/null || exit 2\n\
         timeout -k 2 {OPT_TIMEOUT} \"$B/bin/wasm-opt\" m.wat {s} -o \"$T/r2.wasm\" 2>/dev/null || exit 2\n"
    );
    // A different module: if it also runs differently, say so.
    if let Some(expected) = expected {
        if let Exec::Ok(out) = exec(ctx, &r2) {
            if out != expected {
                let (ea, eb) = (String::from_utf8_lossy(expected).to_string(), String::from_utf8_lossy(&out).to_string());
                let class = match interp::compare(&ea, &eb) {
                    interp::Verdict::Differ(c, _) => c,
                    _ => "other",
                };
                let sig = format!("text-read__mismatch__{class}");
                let msg = format!("the module wasm-opt reads from the standard text of {which}.wasm runs differently in wasmtime: {}", first_diff(expected, &out));
                let repro = format!(
                    "{head}timeout -k 2 {EXEC_TIMEOUT} \"$B/bin/bynexec\" m.wasm > \"$T/a.out\" 2>/dev/null || exit 2\n\
                     timeout -k 2 {EXEC_TIMEOUT} \"$B/bin/bynexec\" \"$T/r2.wasm\" > \"$T/b.out\" 2>/dev/null || exit 2\n\
                     cmp -s \"$T/a.out\" \"$T/b.out\" && exit 0\ndiff \"$T/a.out\" \"$T/b.out\" | head -20\nexit 1\n"
                );
                ctx.finding(&sig, "text-read", &msg, &[("m.wasm", &mwasm), ("m.wat", &mwat), ("r1.wasm", &r1), ("r2.wasm", &r2)], &repro,
                    &[("before.out", ea), ("after.out", eb), ("cmd.txt", cmd)]);
                return;
            }
        }
    }
    let area = text_diff_area(&p1, &p2);
    let sig = format!("text-read__diff__{area}");
    let msg = format!("wasm-opt reads a different module from the standard text (wasmprinter) of {which}.wasm than from its binary; first differing line of wasm-opt -S: {}",
        diff_line(&p1, &p2));
    let repro = format!(
        "{head}cmp -s \"$T/r1.wasm\" \"$T/r2.wasm\" && exit 0\n\
         timeout -k 2 {OPT_TIMEOUT} \"$B/bin/wasm-opt\" \"$T/r1.wasm\" {f} -S 2>/dev/null | grep -v ';; custom section' > \"$T/r1.wat\" || exit 2\n\
         timeout -k 2 {OPT_TIMEOUT} \"$B/bin/wasm-opt\" \"$T/r2.wasm\" {f} -S 2>/dev/null | grep -v ';; custom section' > \"$T/r2.wat\" || exit 2\n\
         cmp -s \"$T/r1.wat\" \"$T/r2.wat\" && exit 0\ndiff \"$T/r1.wat\" \"$T/r2.wat\" | head -20\nexit 1\n"
    );
    ctx.finding(&sig, "text-read", &msg, &[("m.wasm", &mwasm), ("m.wat", &mwat), ("r1.wasm", &r1), ("r2.wasm", &r2)], &repro,
        &[("r1.wat", p1), ("r2.wat", p2), ("cmd.txt", cmd)]);
}

fn diff_line(a: &str, b: &str) -> String {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let i = (0..la.len().max(lb.len())).find(|&i| la.get(i) != lb.get(i)).unwrap_or(0);
    format!("line {}: `{}` vs `{}`", i + 1, la.get(i).unwrap_or(&"").trim(), lb.get(i).unwrap_or(&"").trim())
}

fn one_case(ctx: &mut Ctx, rng: &mut Rng) {
    let input = ctx.tmp.join("input.dat");
    let a = ctx.tmp.join("a.wasm");
    let b = ctx.tmp.join("b.wasm");
    let _ = fs::remove_file(&a);
    let _ = fs::remove_file(&b);

    // Random bytes for -ttf, log-uniform size 256 B .. 16 KiB.
    let len = 256usize << rng.below(7);
    let len = len + rng.below(len);
    let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
    fs::write(&input, &bytes).unwrap();

    // Stack-switching programs (ss.rs): run standalone, or as the initial
    // content that -ttf builds on, so they mix with every other feature.
    let ss_wasm = ctx.tmp.join("ss.wasm");
    let mut standalone = false;
    ctx.ss_case = false;
    ctx.ss_switch = false;
    ctx.smith_case = false;
    let mut smith_features = None;
    let mut smith_nans = false;
    let ss_initial = if rng.chance(0.2) {
        let wat = ctx.tmp.join("ss.wat");
        let text = ss::module(rng);
        ctx.ss_switch = text.contains("(switch ");
        fs::write(&wat, text).unwrap();
        let mut args = vec![wat.display().to_string()];
        args.extend(BASE_FEATURES.iter().map(|s| s.to_string()));
        args.extend(["-o".to_string(), ss_wasm.display().to_string()]);
        let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[]);
        if !o.status.success() {
            // A generator bug unless proven otherwise; keep a few for a look.
            ctx.bump("ss_invalid");
            let n = *ctx.stats.get("ss_invalid").unwrap();
            if n <= 3 {
                let _ = fs::copy(&wat, ctx.out.join(format!("ss-invalid-{n}.wat")));
                let _ = fs::write(ctx.out.join(format!("ss-invalid-{n}.err")), &o.stderr);
            }
            return;
        }
        ctx.bump("ss");
        ctx.ss_case = true;
        standalone = rng.chance(0.5);
        Some(ss_wasm.clone())
    } else {
        None
    };
    // wasm-smith modules (smith.rs): first oracle binary-read (wasm-opt must
    // read and validate every module wasmparser validates), then used as
    // a.wasm directly or as the initial content that -ttf builds on.
    let smith_wasm = ctx.tmp.join("smith.wasm");
    let smith_initial = if ss_initial.is_none() && rng.chance(0.25) {
        let mut features = random_features(rng);
        // wasm-smith emits return_call_ref with tail calls off (wasmparser
        // accepts it under GC alone); binaryen requires tail calls for it.
        if !features.iter().any(|f| f == "--disable-gc") {
            features.retain(|f| f != "--disable-tail-call");
        }
        let Some(bytes) = smith::module(rng, &features) else {
            return ctx.bump("smith_failed");
        };
        if smith::has_declared_elem(&bytes) && rng.chance(0.9) {
            return ctx.bump("smith_throttled_declared_elem");
        }
        if wasmparser::Validator::new_with_features(smith::wasmparser_features(&features)).validate_all(&bytes).is_err() {
            return ctx.bump("smith_invalid");
        }
        fs::write(&smith_wasm, &bytes).unwrap();
        smith_nans = smith::has_nondeterministic_nans(&bytes);
        ctx.bump("smith");
        let mut args = vec![smith_wasm.display().to_string()];
        args.extend(features.iter().cloned());
        let read = ctx.tmp.join("smith-read.wasm");
        args.extend(["-o".to_string(), read.display().to_string()]);
        let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &args, &[]);
        if let Some(kind) = crash_kind(&o) {
            ctx.bump("cases");
            let err = String::from_utf8_lossy(&o.stderr).to_string();
            if READ_LIMITS.iter().any(|m| err.contains(m)) {
                return ctx.bump("skip_binary_read_limitation");
            }
            let sig = format!("crash__{}__binary-read-{}", kind.name(), read_area(kind, &err));
            let repro = crash_repro("smith.wasm", &features, kind, &err);
            let msg = format!("wasm-opt fails to read a valid wasm-smith module ({}): {}", kind.name(), err.lines().rev().take(3).collect::<Vec<_>>().join(" | "));
            let cmd = format!("wasm-opt smith.wasm {}\n", shell_args(&features));
            ctx.finding(&sig, "crash", &msg, &[("smith.wasm", &smith_wasm)], &repro, &[("stderr.txt", err), ("cmd.txt", cmd)]);
            return;
        }
        standalone = rng.chance(0.5);
        ctx.smith_case = standalone;
        smith_features = Some(features);
        Some(smith_wasm.clone())
    } else {
        None
    };
    let initial = if ss_initial.is_some() {
        ss_initial
    } else if smith_initial.is_some() {
        smith_initial
    } else if !ctx.seeds.is_empty() && rng.chance(0.5) {
        Some(ctx.seeds[rng.below(ctx.seeds.len())].clone())
    } else {
        None
    };
    // Seeds may use any feature of the base set, so never disable more for them.
    let features = if let Some(f) = smith_features {
        f
    } else if initial.is_some() {
        BASE_FEATURES.iter().map(|s| s.to_string()).collect()
    } else {
        random_features(rng)
    };
    let closed_world = rng.chance(0.3);
    let has_eh = !features.iter().any(|f| f == "--disable-exception-handling");

    // 1. generate
    let mut gen_args = vec![input.display().to_string(), "-ttf".to_string()];
    if let Some(i) = &initial {
        gen_args.push(format!("--initial-fuzz={}", i.display()));
        ctx.bump("seeded");
    }
    gen_args.extend(features.iter().cloned());
    if closed_world {
        gen_args.push("--closed-world".into());
    }
    // NaN bits are nondeterministic and leak through reinterpret/extract_lane,
    // so, like fuzz_opt.py (NANS=False -> --denan), only de-NaN'd modules get
    // the exec comparison; the rest get the crash oracle only.
    let denan = (standalone && !(ctx.smith_case && smith_nans)) || (!standalone && rng.chance(0.8));
    if denan {
        gen_args.push("--denan".into());
    }
    if has_eh {
        gen_args.push("--translate-to-exnref".into());
    }
    let mut gen_tail = gen_args[1..].to_vec();
    gen_args.push("-o".into());
    gen_args.push(a.display().to_string());
    ctx.bump("cases");
    let o = if standalone && ctx.smith_case {
        ctx.bump("smith_standalone");
        fs::copy(&smith_wasm, &a).unwrap();
        gen_tail = vec!["(wasm-smith module)".to_string()];
        None
    } else if standalone {
        ctx.bump("ss_standalone");
        fs::copy(&ss_wasm, &a).unwrap();
        gen_tail = vec!["(ss.rs module)".to_string()];
        None
    } else {
        Some(run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &gen_args, &[]))
    };
    if let Some(kind) = o.as_ref().and_then(crash_kind) {
        let o = o.as_ref().unwrap();
        let err = String::from_utf8_lossy(&o.stderr).to_string();
        if kind == CrashKind::Exit && !err.contains("Fatal") {
            ctx.bump("skip_gen_error");
            return;
        }
        if initial.as_deref() == Some(smith_wasm.as_path()) && READ_LIMITS.iter().any(|m| err.contains(m)) {
            return ctx.bump("skip_binary_read_limitation");
        }
        let area = if kind == CrashKind::Hang { "ttf".to_string() } else { failing_pass(ctx, &gen_args) };
        let area = if area == "io" { "ttf".into() } else { area };
        let sig = format!("crash__{}__{}", kind.name(), area);
        for a in gen_tail.iter_mut() {
            if a.starts_with("--initial-fuzz=") {
                *a = "--initial-fuzz=initial.wasm".into();
            }
        }
        let repro = crash_repro("input.dat", &gen_tail, kind, &err);
        let msg = format!("wasm-opt -ttf failed ({}): {}", kind.name(), err.lines().rev().take(3).collect::<Vec<_>>().join(" | "));
        let mut files: Vec<(&str, &Path)> = vec![("input.dat", &input)];
        if let Some(i) = &initial {
            files.push(("initial.wasm", i));
        }
        ctx.finding(&sig, "crash", &msg, &files, &repro, &[("stderr.txt", err.clone())]);
        return;
    }

    // 2. baseline run
    let expected = if !denan {
        Vec::new()
    } else {
        match exec(ctx, &a) {
            Exec::Ok(out) => out,
            Exec::Limit => return ctx.bump("skip_a_host_limit"),
            Exec::Reject(_) => return ctx.bump("skip_a_rejected"),
            Exec::Timeout => return ctx.bump("skip_a_timeout"),
            Exec::Crash(_) => return ctx.bump("skip_a_engine_crash"),
        }
    };
    // Oracle binary-read, exec part: the module as binaryen read and wrote it
    // back (no passes) must run like the wasm-smith original.
    if denan && ctx.smith_case {
        let read = ctx.tmp.join("smith-read.wasm");
        if let Exec::Ok(out) = exec(ctx, &read) {
            if out != expected {
                let msg = format!("wasm-opt a.wasm {} (no passes) changed wasmtime output: {}", shell_args(&features), first_diff(&expected, &out));
                let repro = exec_repro(&features, &[], "mismatch");
                let cmd = format!("wasm-opt a.wasm {}\n", shell_args(&features));
                ctx.finding("exec__mismatch__binary-read", "exec", &msg, &[("a.wasm", &a), ("b.wasm", &read)], &repro,
                    &[("before.out", String::from_utf8_lossy(&expected).to_string()), ("after.out", String::from_utf8_lossy(&out).to_string()), ("cmd.txt", cmd)]);
                return;
            }
            ctx.bump("smith_read_same");
        }
    }
    // Oracle text-read on the generated module (ss.rs cases are text already).
    if !ctx.ss_case && rng.chance(0.3) {
        text_read(ctx, rng, &a, &features, if denan { Some(&expected) } else { None }, "a");
    }
    if denan {
        let gencmd = format!("wasm-opt input.dat {}", shell_args(&gen_tail));
        interp_oracle(ctx, &a, &features, &expected, "generated", &gencmd);
        if has_eh && !standalone && !ctx.ss_switch && rng.chance(0.5) {
            pre_exnref_oracle(ctx, &gen_args[..gen_args.len() - 2], &features, &a, &expected);
        }
    }

    // 3. optimize
    let pipeline = random_pipeline(rng, closed_world, &features);
    let mut opt_args = vec![a.display().to_string()];
    opt_args.extend(features.iter().cloned());
    opt_args.extend(pipeline.iter().cloned());
    opt_args.push("-o".into());
    opt_args.push(b.display().to_string());
    let o = run_timeout(OPT_TIMEOUT, &ctx.bin.join("wasm-opt"), &opt_args, &[]);
    let cmdline = format!("wasm-opt a.wasm {} {}", shell_args(&features), shell_args(&pipeline));
    if let Some(kind) = crash_kind(&o) {
        let err = String::from_utf8_lossy(&o.stderr).to_string();
        if matches!(kind, CrashKind::Exit | CrashKind::Fatal) && LOWER_LIMITS.iter().any(|m| err.contains(m)) {
            return ctx.bump("skip_lowering_limitation");
        }
        let area = if kind == CrashKind::Hang {
            sanitize(pipeline.first().map(|s| s.as_str()).unwrap_or("unknown"))
        } else {
            failing_pass(ctx, &opt_args)
        };
        let sig = format!("crash__{}__{}", kind.name(), area);
        let mut tail = features.clone();
        tail.extend(pipeline.iter().cloned());
        let repro = crash_repro("a.wasm", &tail, kind, &err);
        let msg = format!("{cmdline} failed ({}): {}", kind.name(), err.lines().rev().take(3).collect::<Vec<_>>().join(" | "));
        ctx.finding(&sig, "crash", &msg, &[("a.wasm", &a)], &repro, &[("stderr.txt", err.clone()), ("cmd.txt", cmdline + "\n")]);
        return;
    }

    let roundtrip = rng.chance(0.25);
    let crash_only = pipeline.iter().any(|f| CRASH_ONLY_PASSES.contains(&f.as_str()));
    if !denan || crash_only {
        if roundtrip {
            text_roundtrip(ctx, &b, &features, None, &cmdline);
        }
        ctx.bump(if crash_only { "exec_skipped_crash_only" } else { "exec_skipped_nans" });
        ctx.bump("pass");
        return;
    }

    // 4. optimized run
    match exec(ctx, &b) {
        Exec::Ok(out) => {
            if out == expected {
                if roundtrip {
                    text_roundtrip(ctx, &b, &features, Some(&out), &cmdline);
                } else if rng.chance(0.15) {
                    text_read(ctx, rng, &b, &features, Some(&out), "b");
                }
                // binaryen's interpreter would also call asyncify's exports,
                // which bynexec leaves out.
                if rng.chance(0.3) && !pipeline.iter().any(|f| f == "--asyncify") {
                    interp_oracle(ctx, &b, &features, &out, "optimized", &cmdline);
                }
                ctx.bump("pass");
                return;
            }
            let area = blame_pass(ctx, &a, &features, &pipeline, &expected).unwrap_or_else(|| {
                sanitize(pipeline.iter().find(|f| f.starts_with('-') && !f.starts_with("--pass-arg")).map(|s| s.as_str()).unwrap_or("unknown"))
            });
            // A lowering pass that drops a trap (e.g. by wrapping a 64-bit
            // address or delta to 32 bits) gets its own kind, so that other
            // wrong results of the same pass stay visible.
            let lowering = LOWER_GROUPS.iter().any(|(g, _)| g.iter().any(|p| sanitize(p) == area));
            let kind = if lowering && first_diff_is_trap(&expected, &out) { "traplost" } else { "mismatch" };
            if kind == "traplost" && area == "memory64-lowering" && wrapped_address_trap(ctx, &a, &features, &expected, &out) {
                return ctx.bump("skip_memory64_lowering_wrapped_address");
            }
            if area == "memory64-lowering" && masked_matches(ctx, &a, &features, &out) {
                return ctx.bump("skip_memory64_lowering_wrapped_operand");
            }
            let sig = format!("exec__{kind}__{area}");
            let msg = format!("{cmdline}: wasmtime output changed: {}", first_diff(&expected, &out));
            let repro = exec_repro(&features, &pipeline, "mismatch");
            let ea = String::from_utf8_lossy(&expected).to_string();
            let eb = String::from_utf8_lossy(&out).to_string();
            ctx.finding(&sig, "exec", &msg, &[("a.wasm", &a), ("b.wasm", &b)], &repro,
                &[("before.out", ea), ("after.out", eb), ("cmd.txt", cmdline + "\n")]);
        }
        Exec::Limit => ctx.bump("skip_b_host_limit"),
        Exec::Timeout => ctx.bump("skip_b_timeout"),
        Exec::Reject(err) => {
            let area = sanitize(pipeline.first().map(|s| s.as_str()).unwrap_or("unknown"));
            let sig = format!("exec__reject__{area}");
            let msg = format!("{cmdline}: wasmtime rejects the optimized module: {}", err.lines().take(4).collect::<Vec<_>>().join(" | "));
            let repro = exec_repro(&features, &pipeline, "reject");
            ctx.finding(&sig, "exec", &msg, &[("a.wasm", &a), ("b.wasm", &b)], &repro, &[("cmd.txt", cmdline + "\n")]);
        }
        Exec::Crash(err) => {
            let area = sanitize(pipeline.first().map(|s| s.as_str()).unwrap_or("unknown"));
            let sig = format!("exec__enginecrash__{area}");
            let msg = format!("{cmdline}: wasmtime crashed on the optimized module: {}", err.lines().take(4).collect::<Vec<_>>().join(" | "));
            let repro = exec_repro(&features, &pipeline, "crash");
            ctx.finding(&sig, "exec", &msg, &[("a.wasm", &a), ("b.wasm", &b)], &repro, &[("cmd.txt", cmdline + "\n")]);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && args[1] == "--ss" {
        // Print one ss.rs module (for looking at the generator).
        let mut rng = Rng(args[2].parse().expect("seed"));
        print!("{}", ss::module(&mut rng));
        return;
    }
    if args.len() == 3 && args[1] == "--mask64" {
        // Print a module text (wasm-opt -S) with its 64-bit operands masked (masked_matches).
        let wat = fs::read_to_string(&args[2]).expect("read");
        print!("{}", mask64::mask(&wat).expect("parse"));
        return;
    }
    if args.len() == 5 && args[1] == "--wat" {
        // Print a module as wasmprinter text (oracle text-read): --wat <m.wasm> <fold 0|1> <names 0|1>.
        let bytes = fs::read(&args[2]).expect("read");
        print!("{}", wat::print(&bytes, args[3] == "1", args[4] == "1").expect("print"));
        return;
    }
    if args.len() >= 4 && args[1] == "--interp-check" {
        std::process::exit(interp_check(&args[2..]));
    }
    if args.len() != 5 {
        eprintln!("usage: bynfuzz <build-dir> <seed> <seconds> <out-dir>");
        std::process::exit(2);
    }
    let build = fs::canonicalize(&args[1]).expect("build dir");
    let seed: u64 = args[2].parse().expect("seed");
    let secs: u64 = args[3].parse().expect("seconds");
    let out = PathBuf::from(&args[4]);
    fs::create_dir_all(out.join("findings")).unwrap();
    let out = fs::canonicalize(out).unwrap();
    let tmp = out.join("tmp");
    fs::create_dir_all(&tmp).unwrap();
    let mut ctx = Ctx {
        bin: build.join("bin"),
        out,
        tmp,
        stats: BTreeMap::new(),
        per_sig: BTreeMap::new(),
        seed,
        start: Instant::now(),
        seeds: Vec::new(),
        ss_case: false,
        ss_switch: false,
        smith_case: false,
    };
    // Snapshot the seeds into our own tmp: build.sh regenerates build-dir/seeds
    // while searches run against the same build, and a seed that vanishes
    // mid-search made -ttf fail with "Failed opening" (a false crash__fatal__ttf).
    if let Ok(rd) = fs::read_dir(build.join("seeds")) {
        let snap = ctx.tmp.join("seeds");
        let _ = fs::create_dir_all(&snap);
        let mut paths: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "wasm")).collect();
        paths.sort();
        for p in paths {
            let dst = snap.join(p.file_name().unwrap());
            if fs::copy(&p, &dst).is_ok() {
                ctx.seeds.push(dst);
            }
        }
    }
    for k in ["cases", "failures"] {
        ctx.stats.insert(k.into(), 0);
    }
    ctx.write_stats();
    let mut rng = Rng(seed ^ 0x5eed_b1a2_e000_0000);
    let deadline = ctx.start + Duration::from_secs(secs);
    let mut last = Instant::now();
    while Instant::now() < deadline {
        one_case(&mut ctx, &mut rng);
        if last.elapsed() > Duration::from_secs(2) {
            ctx.write_stats();
            last = Instant::now();
        }
    }
    ctx.write_stats();
    let _ = fs::remove_dir_all(&ctx.tmp);
}
