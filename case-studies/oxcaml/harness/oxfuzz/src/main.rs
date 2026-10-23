// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! oxfuzz: differential tester for the oxcaml compiler.
//!
//!   oxfuzz gen <seed>                       print one generated program
//!   oxfuzz run <build> <seed> <secs> <out>  search loop (harness run.sh)
//!
//! Oracles:
//! - diff: native (flambda2, several flag sets) vs bytecode (ocamlc) on the
//!   same deterministic program: stdout and exit status must agree.
//! - crash: the native compiler fails (fatal error, assertion, uncaught
//!   exception, signal, timeout) on a program that ocamlc accepts.

mod gen;

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const LIBS: &[&str] = &["-w", "-a", "-I", "+stdlib_stable", "-I", "+stdlib_upstream_compatible"];
const COMPILE_TIMEOUT: u64 = 120;
const RUN_TIMEOUT: u64 = 5;
const CAP_PER_SIG: usize = 3;

/// Native configurations. The first two run on every case; one of the rest
/// rotates in per case.
const CONFIGS: &[(&str, &str)] = &[
    ("O3", "-O3 -dflambda-invariants -runtime-variant d"),
    ("Oclassic", "-Oclassic"),
    ("default", ""),
    ("O4", "-O4"),
    (
        "O3x",
        "-O3 -flambda2-match-in-match -flambda2-expert-can-inline-recursive-functions -flambda2-join-algorithm n-way -flambda2-result-types-all-functions",
    ),
    ("O2gi", "-O2 -regalloc gi"),
    ("O3irc", "-O3 -regalloc irc"),
    ("O3ls", "-O3 -regalloc ls"),
];

/// Toggle oracle: base levels and code-generation switches that must not change
/// what a program prints. Most are off by default (so the regular configs never
/// reach that code: SSA, x86 peephole, block merging/layout, internal
/// assembler, vectorizer, DWARF, long frames, branch relaxation), the rest turn
/// a default pass off or push a middle-end knob to an extreme. `{s}` is
/// replaced by a per-case number.
const TOGGLE_BASES: &[&str] = &["-O3", "-O3", "", "-O2", "-O3 -flambda2-reaper", "-Oclassic"];
const TOGGLES: &[(&str, &str)] = &[
    ("use_ssa", "-use-ssa"),
    ("use_ssa_novalidate", "-use-ssa -no-ssa-validate"),
    ("use_ssa_nosimplify", "-use-ssa -no-ssa-simplify"),
    ("x86_peephole", "-x86-peephole-optimize"),
    ("cfg_merge_blocks", "-cfg-merge-blocks"),
    ("cfg_block_layout", "-cfg-block-layout"),
    ("cfg_copy_propagation", "-cfg-copy-propagation"),
    ("reorder_blocks_random", "-reorder-blocks-random {s}"),
    ("internal_assembler", "-internal-assembler"),
    ("vectorize", "-vectorize"),
    ("dwarf", "-g"),
    ("dwarf_alter_codegen", "-g -gdwarf-may-alter-codegen -gdwarf-may-alter-codegen-experimental"),
    ("cfg_invariants", "-dcfg-invariants"),
    ("long_frames", "-debug-long-frames-threshold 0"),
    ("branch_relaxation", "-dbranch-relaxation-max-displacement {s}"),
    ("function_layout", "-function-layout source"),
    ("no_shrink_wrap", "-no-cfg-prologue-shrink-wrap"),
    ("shrink_wrap_threshold", "-cfg-prologue-shrink-wrap-threshold 1"),
    ("no_value_propagation", "-no-cfg-value-propagation"),
    ("no_cfg_peephole", "-no-cfg-peephole-optimize"),
    ("no_dead_trap_elim", "-no-cfg-eliminate-dead-trap-handlers"),
    ("stack_checks_threshold", "-cfg-stack-checks-threshold 0"),
    ("caml_apply_fast_path", "-caml-apply-inline-fast-path"),
    ("advanced_meet", "-flambda2-advanced-meet"),
    ("no_join_points", "-no-flambda2-join-points"),
    ("no_unbox_control_flow", "-no-flambda2-unbox-along-intra-function-control-flow"),
    ("max_unboxing_depth", "-flambda2-expert-max-unboxing-depth 0"),
    ("cont_specialization", "-flambda2-expert-cont-specialization-threshold 1000"),
    ("cmm_safe_subst", "-flambda2-expert-cmm-safe-subst"),
    ("inline_effects_in_cmm", "-flambda2-expert-inline-effects-in-cmm"),
    ("phantom_lets", "-flambda2-expert-phantom-lets"),
    ("backend_cse_toplevel", "-flambda2-backend-cse-at-toplevel"),
    ("inline_max_depth", "-flambda2-inline-max-depth 0"),
    ("inline_huge", "-flambda2-inline-threshold 1000 -flambda2-inline-large-function-size 1000 -flambda2-inline-max-depth 6"),
    ("cse_depth", "-flambda2-cse-depth 0"),
    ("join_depth", "-flambda2-join-depth 0"),
    ("no_simplify_stubs", "-no-flambda2-simplify-stubs"),
];
/// -flambda2-expert-inline-effects-in-cmm fails on most programs it is
/// given (two distinct crashes; 2/249 burn cases at full weight, 3/640 at 1/8), which pushed the clean
/// failure rate over 1%.
const RARE_TOGGLES: &[&str] = &["inline_effects_in_cmm"];
// Not used: -flambda2-kind-checks fails with "Unexpected approximation found when
// block approximation was expected" on ~75% of generated programs (checker noise).

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

fn generate(seed: u64) -> gen::Program {
    let mut src = ephyra::Source::new(seed, 10);
    let mut g = gen::Gen::new(&mut src);
    let nf = 3 + (seed % 6) as usize;
    let nt = 8 + (seed / 7 % 10) as usize;
    g.program(nf, nt, &[])
}

struct Out {
    code: i32, // exit code; 124 = timeout; 128+n = signal
    stdout: String,
    stderr: String,
}

fn run(cmd: &[String], dir: &Path, secs: u64) -> Out {
    let mut c = Command::new("timeout");
    c.arg("-k").arg("2").arg(secs.to_string()).args(cmd).current_dir(dir);
    // backtraces for uncaught exceptions in the compiler (signature naming)
    c.env("OCAMLRUNPARAM", "b");
    c.stdin(Stdio::null());
    match c.output() {
        Ok(o) => {
            use std::os::unix::process::ExitStatusExt;
            let code = o.status.code().unwrap_or_else(|| 128 + o.status.signal().unwrap_or(0));
            Out {
                code,
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            }
        }
        Err(e) => Out { code: -1, stdout: String::new(), stderr: e.to_string() },
    }
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| w.len() > 1)
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

/// Longest prefix without digits, used as a fixed grep string in repro.sh.
fn stable_prefix(line: &str) -> String {
    let p: String = line.chars().take_while(|c| !c.is_ascii_digit()).collect();
    let p = p.trim_end().to_string();
    p.chars().take(80).collect()
}

/// Classify a native compiler failure: (kind, area, grep-key)
fn classify_crash(o: &Out) -> (String, String, String) {
    if o.code == 124 || o.code == 137 {
        return ("timeout".into(), "compile".into(), String::new());
    }
    let err = &o.stderr;
    let lines: Vec<&str> = err.lines().collect();
    if err.contains("Cmm invariants failed") || err.contains("Cmm invariant") {
        let l = lines.iter().find(|l| !l.trim().is_empty()).copied().unwrap_or("");
        let w = words(l);
        let area = w.iter().take(4).cloned().collect::<Vec<_>>().join("_");
        return ("cmm_invariant".into(), area, stable_prefix(l));
    }
    if let Some(l) = lines.iter().find(|l| l.contains("Assert_failure") || l.contains("assertion")) {
        // Assert_failure("path/file.ml", line, col)
        let area = if let Some(i) = l.find("Assert_failure(\"") {
            let rest = &l[i + 16..];
            let path = rest.split('"').next().unwrap_or("");
            Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        } else {
            words(l).into_iter().take(4).collect::<Vec<_>>().join("_")
        };
        return ("assert".into(), sanitize(&area), stable_prefix(l.trim()));
    }
    if err.contains("Assembler error") || err.contains("Assembler messages") {
        // "<tmp>.s:<line>: Error: <message>": keep only the message words
        let l = lines.iter().find(|l| l.contains(": Error: ")).copied().unwrap_or("");
        let msg = l.split(": Error: ").nth(1).unwrap_or("");
        let area = words(msg).into_iter().take(4).collect::<Vec<_>>().join("_");
        return ("assembler".into(), sanitize(&area), stable_prefix(msg));
    }
    if let Some(l) = lines.iter().find(|l| l.contains("Fatal error")) {
        let l = l.trim();
        let mut msg = l.split("Fatal error").nth(1).unwrap_or("").trim_start_matches(':').trim().to_string();
        let mut key_line = l.to_string();
        if msg.is_empty() {
            // message on the following line
            if let Some(i) = lines.iter().position(|x| x.trim() == l) {
                if let Some(n) = lines.get(i + 1) {
                    msg = n.trim().to_string();
                    key_line = n.trim().to_string();
                }
            }
        }
        if let Some(exn) = msg.strip_prefix("exception ") {
            // uncaught exception in the compiler: name it by the exception and the
            // innermost compiler frame of the backtrace (OCAMLRUNPARAM=b)
            let name = exn.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.').next().unwrap_or("");
            let name = name.rsplit('.').next().unwrap_or(name);
            let area = match frame(&lines) {
                Some(f) => format!("{}_at_{}", name, f),
                None => name.to_string(),
            };
            return ("exception".into(), sanitize(&area), stable_prefix(&key_line));
        }
        let area = words(&msg).into_iter().take(4).collect::<Vec<_>>().join("_");
        return ("fatal".into(), sanitize(&area), stable_prefix(&key_line));
    }
    if let Some(l) = lines.iter().find(|l| l.contains("exception") || l.contains("Exception")) {
        let w = words(l);
        let area = w.into_iter().filter(|x| x != "uncaught" && x != "exception" && x != "fatal" && x != "error").take(3).collect::<Vec<_>>().join("_");
        return ("exception".into(), sanitize(&area), stable_prefix(l.trim()));
    }
    if o.code > 128 {
        return ("signal".into(), "compiler".into(), String::new());
    }
    let l = lines.iter().find(|l| l.starts_with("Error") || l.contains("rror")).copied().unwrap_or("");
    let area = words(l).into_iter().take(4).collect::<Vec<_>>().join("_");
    ("error".into(), sanitize(&area), stable_prefix(l.trim()))
}

/// First backtrace frame outside the stdlib: "Called from Cfg_selectgen.Make.emit_expr_raise in file ..."
fn frame(lines: &[&str]) -> Option<String> {
    for l in lines {
        let l = l.trim();
        let Some(rest) = l.strip_prefix("Called from ").or_else(|| l.strip_prefix("Raised at ")) else { continue };
        let f = rest.split(' ').next().unwrap_or("");
        if f.starts_with("Stdlib") || f.starts_with("Misc.") || f.starts_with("CamlinternalFormat") || f.is_empty() {
            continue;
        }
        let parts: Vec<&str> = f.split('.').filter(|p| *p != "Make" && !p.starts_with('(')).collect();
        let m = parts.first().copied().unwrap_or("");
        let func = parts.last().copied().unwrap_or("");
        return Some(format!("{}_{}", m, func).to_ascii_lowercase());
    }
    None
}

fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let t = t.trim_matches('_').to_string();
    if t.is_empty() { "unknown".into() } else { t }
}

struct Ctx {
    bin: PathBuf,
    ocamlopt: String,
    out: PathBuf,
    work: PathBuf,
    cases: u64,
    failures: u64,
    extra: BTreeMap<String, u64>,
    start: Instant,
}

impl Ctx {
    fn bump(&mut self, k: &str) {
        *self.extra.entry(k.to_string()).or_insert(0) += 1;
    }

    fn write_stats(&self) {
        let mut s = format!("{{\"cases\": {}, \"failures\": {}", self.cases, self.failures);
        for (k, v) in &self.extra {
            s += &format!(", \"{k}\": {v}");
        }
        let el = self.start.elapsed().as_secs_f64();
        s += &format!(", \"cases_per_sec\": {:.3}}}\n", self.cases as f64 / el.max(0.001));
        let tmp = self.out.join("stats.json.tmp");
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, self.out.join("stats.json"));
        }
    }

    fn ocamlc(&self) -> String {
        self.bin.join("ocamlc.opt").to_string_lossy().into_owned()
    }

    #[allow(clippy::too_many_arguments)]
    fn record(&mut self, sig: &str, oracle: &str, message: &str, seed: u64, prog: &str, flags: &str, kind: ReproKind) -> bool {
        let dir = self.out.join("findings").join(sig);
        let n = fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
        if n >= CAP_PER_SIG {
            return false;
        }
        let d = dir.join(n.to_string());
        let _ = fs::create_dir_all(&d);
        let _ = fs::write(d.join("prog.ml"), prog);
        let fj = format!(
            "{{\"signature\": {}, \"oracle\": {}, \"message\": {}, \"seed\": {}, \"flags\": {}}}\n",
            json_str(sig),
            json_str(oracle),
            json_str(message),
            seed,
            json_str(flags)
        );
        let _ = fs::write(d.join("finding.json"), fj);
        let script = repro_script(sig, flags, &kind);
        let p = d.join("repro.sh");
        let _ = fs::write(&p, script);
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o755));
        true
    }
}

enum ReproKind {
    Diff,
    Crash { key: String, timeout: bool },
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o += "\\\"",
            '\\' => o += "\\\\",
            '\n' => o += "\\n",
            '\t' => o += "\\t",
            c if (c as u32) < 0x20 => o += &format!("\\u{:04x}", c as u32),
            c => o.push(c),
        }
    }
    o + "\""
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn repro_script(sig: &str, flags: &str, kind: &ReproKind) -> String {
    let mut s = String::new();
    s += "#!/bin/bash\n";
    s += &format!("# oxfuzz finding: {sig}\n");
    s += "# usage: repro.sh <build-dir>   exit 1 = reproduces, 0 = does not, 2 = cannot decide\n";
    s += "B=$(realpath \"$1\") || exit 2\nHERE=$(cd \"$(dirname \"$0\")\" && pwd)\nI=\"$B/install/bin\"\n";
    s += &format!("FLAGS=({flags})\n");
    s += "LIBS=(-w -a -I +stdlib_stable -I +stdlib_upstream_compatible)\n";
    s += "T=$(mktemp -d)\ntrap 'rm -rf \"$T\"' EXIT\ncp \"$HERE/prog.ml\" \"$T/prog.ml\"\ncd \"$T\" || exit 2\n";
    s += "\"$I/ocamlc.opt\" \"${LIBS[@]}\" stdlib_stable.cma stdlib_upstream_compatible.cma prog.ml -o p.byte >/dev/null 2>&1 || exit 2\n";
    match kind {
        ReproKind::Diff => {
            s += &format!("timeout -k 2 {} ./p.byte > ref.out 2>/dev/null; rc=$?\n", RUN_TIMEOUT * 3);
            s += "[ $rc -eq 124 ] && exit 2\n";
            s += &format!("timeout -k 2 {} \"$I/ocamlopt.opt\" \"${{FLAGS[@]}}\" \"${{LIBS[@]}}\" stdlib_stable.cmxa stdlib_upstream_compatible.cmxa prog.ml -o p.exe > c.err 2>&1 || exit 2\n", COMPILE_TIMEOUT * 2);
            s += &format!("timeout -k 2 {} ./p.exe > nat.out 2>/dev/null; nrc=$?\n", RUN_TIMEOUT * 3);
            s += "if [ \"$rc\" != \"$nrc\" ] || ! cmp -s ref.out nat.out; then\n  diff ref.out nat.out | head -5; echo \"exit: bytecode $rc native $nrc\"; exit 1\nfi\nexit 0\n";
        }
        ReproKind::Crash { key, timeout } => {
            s += &format!("timeout -k 2 {} \"$I/ocamlopt.opt\" \"${{FLAGS[@]}}\" \"${{LIBS[@]}}\" stdlib_stable.cmxa stdlib_upstream_compatible.cmxa prog.ml -o p.exe > c.err 2>&1; rc=$?\n", COMPILE_TIMEOUT * 2);
            s += "[ $rc -eq 0 ] && exit 0\n";
            if *timeout {
                s += "[ $rc -eq 124 ] || [ $rc -eq 137 ] && exit 1\nexit 0\n";
            } else if key.is_empty() {
                s += "[ $rc -gt 128 ] && exit 1\nexit 0\n";
            } else {
                s += &format!("if grep -qF -- {} c.err; then head -5 c.err; exit 1; fi\nexit 0\n", sh_quote(key));
            }
        }
    }
    s
}

/// Kind of a native-vs-bytecode difference. Native runs that hit an
/// flambda2 `Invalid` term (code the optimiser proved unreachable) abort with
/// "[flambda2] Invalid code" on stderr: a distinct cause from other crashes.
fn diff_kind(r: &Out, n: &Out) -> &'static str {
    if n.code != r.code && (n.code > 128 || n.code == 124) {
        if n.stderr.contains("[flambda2] Invalid code") { "invalid_code" } else { "crash" }
    } else if n.stdout == r.stdout {
        "exit"
    } else {
        "output"
    }
}

/// Which test does the first differing line belong to?
fn diff_area(r: &str, n: &str, foci: &[gen::Focus]) -> String {
    let rl: Vec<&str> = r.lines().collect();
    let nl: Vec<&str> = n.lines().collect();
    for i in 0..rl.len().max(nl.len()) {
        let a = rl.get(i).copied();
        let b = nl.get(i).copied();
        if a != b {
            let l = a.or(b).unwrap_or("");
            let name = l.split(' ').next().unwrap_or("");
            if foci.is_empty() {
                return "oxfuzzer".into();
            }
            if let Some(k) = name.strip_prefix('t').and_then(|k| k.parse::<usize>().ok()) {
                if let Some(f) = foci.get(k) {
                    return f.name().into();
                }
            }
            if name == "cnt" {
                return "global_counter".into();
            }
            return "unknown".into();
        }
    }
    "unknown".into()
}

fn one_case(cx: &mut Ctx, base_seed: u64, i: u64) {
    let seed = splitmix(base_seed.wrapping_mul(1_000_003).wrapping_add(i));
    let dir = cx.work.clone();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::create_dir_all(&dir);
    // 15% of cases: upstream's own generator, under configs it does not run
    let use_oxf = seed % 100 < 15 && cx.bin.join("oxfuzzer").exists();
    let (prog, foci) = if use_oxf {
        let o = run(&[cx.bin.join("oxfuzzer").to_string_lossy().into(), "-seed".into(), (seed % 1_000_000_000).to_string()], &dir, 30);
        if o.code != 0 || o.stdout.is_empty() {
            cx.bump("oxfuzzer_fail");
            return;
        }
        cx.bump("oxfuzzer_cases");
        (o.stdout, Vec::new())
    } else {
        let p = generate(seed);
        (p.text, p.foci)
    };
    if fs::write(dir.join("prog.ml"), &prog).is_err() {
        cx.bump("scratch_write_error");
        return;
    }
    let libs: Vec<String> = LIBS.iter().map(|s| s.to_string()).collect();
    // bytecode reference
    let mut cmd = vec![cx.ocamlc()];
    cmd.extend(libs.iter().cloned());
    cmd.extend(["stdlib_stable.cma", "stdlib_upstream_compatible.cma", "prog.ml", "-o", "p.byte"].map(String::from));
    let o = run(&cmd, &dir, COMPILE_TIMEOUT);
    if o.code != 0 {
        cx.bump("gen_reject");
        let rej = cx.out.join("rejects");
        let _ = fs::create_dir_all(&rej);
        let n = fs::read_dir(&rej).map(|d| d.count()).unwrap_or(0);
        if n < 20 {
            let _ = fs::write(rej.join(format!("{seed}.ml")), &prog);
            let _ = fs::write(rej.join(format!("{seed}.err")), &o.stderr);
        }
        return;
    }
    let r = run(&["./p.byte".into()], &dir, RUN_TIMEOUT);
    if r.code == 124 || r.code == 137 {
        cx.bump("bytecode_timeout");
        return;
    }
    if r.code != 0 {
        cx.bump("bytecode_nonzero_exit");
    }
    cx.cases += 1;
    // configs: two fixed plus one rotating
    let rot = 2 + (seed / 3 % (CONFIGS.len() as u64 - 2)) as usize;
    let mut sigs: HashSet<String> = HashSet::new();
    let mut failed = false;
    // -Oclassic on half the cases: it has two known crashes (README) that would
    // otherwise sit near the 1% failure-rate limit on their own
    let classic = if seed / 5 % 2 == 0 { 1 } else { 2 + ((rot - 2 + 1) % (CONFIGS.len() - 2)) };
    for &ci in &[0usize, classic, rot] {
        let (cname, flags) = CONFIGS[ci];
        let exe = format!("p_{cname}.exe");
        let mut cmd = vec![cx.ocamlopt.clone()];
        cmd.extend(flags.split_whitespace().map(String::from));
        cmd.append(&mut libs.clone());
        cmd.extend(["stdlib_stable.cmxa", "stdlib_upstream_compatible.cmxa", "prog.ml", "-o", &exe].map(String::from));
        let t0 = Instant::now();
        let c = run(&cmd, &dir, COMPILE_TIMEOUT);
        let _ = t0;
        if c.code != 0 {
            let (kind, area, key) = classify_crash(&c);
            let sig = format!("crash__{kind}__{area}");
            if sigs.insert(sig.clone()) {
                let msg = format!("native compile failed with {cname} ({flags}), exit {}: {}", c.code, c.stderr.lines().take(6).collect::<Vec<_>>().join(" | "));
                cx.record(&sig, "crash", &msg, seed, &prog, flags, ReproKind::Crash { key, timeout: kind == "timeout" });
                failed = true;
            }
            continue;
        }
        let n = run(&[format!("./{exe}")], &dir, RUN_TIMEOUT);
        if n.code == r.code && n.stdout == r.stdout {
            continue;
        }
        // confirm: both binaries again; a difference that does not repeat is
        // counted as flaky (timing, load), not as a failure
        let r2 = run(&["./p.byte".into()], &dir, RUN_TIMEOUT * 2);
        let n2 = run(&[format!("./{exe}")], &dir, RUN_TIMEOUT * 2);
        if r2.code != r.code || r2.stdout != r.stdout || n2.code != n.code || n2.stdout != n.stdout {
            cx.bump("flaky_diff");
            continue;
        }
        if n.code == 124 && r.code == 0 {
            // native hang while bytecode terminates
        }
        let kind = diff_kind(&r, &n);
        let area = diff_area(&r.stdout, &n.stdout, &foci);
        let sig = format!("diff__{kind}__{area}");
        if sigs.insert(sig.clone()) {
            let msg = format!(
                "{cname} ({flags}) differs from bytecode: exit {} vs {}; first diff: {}",
                n.code,
                r.code,
                first_diff(&r.stdout, &n.stdout)
            );
            cx.record(&sig, "diff", &msg, seed, &prog, flags, ReproKind::Diff);
            failed = true;
        }
    }
    if toggle_case(cx, &dir, seed, &prog, &r, &foci, &mut sigs) {
        failed = true;
    }
    if failed {
        cx.failures += 1;
    }
}

/// Result of one native compile + run, compared with the bytecode run `r`.
enum Check {
    Same,
    Crash { kind: String, area: String, key: String, msg: String },
    Diff { kind: &'static str, area: String, msg: String },
}

impl Check {
    fn sig(&self, oracle_diff: &str) -> Option<String> {
        match self {
            Check::Same => None,
            Check::Crash { kind, area, .. } => Some(format!("crash__{kind}__{area}")),
            Check::Diff { kind, area, .. } => Some(format!("{oracle_diff}__{kind}__{area}")),
        }
    }
}

fn check_native(cx: &mut Ctx, dir: &Path, flags: &str, exe: &str, r: &Out, foci: &[gen::Focus]) -> Check {
    let mut cmd = vec![cx.ocamlopt.clone()];
    cmd.extend(flags.split_whitespace().map(String::from));
    cmd.extend(LIBS.iter().map(|s| s.to_string()));
    cmd.extend(["stdlib_stable.cmxa", "stdlib_upstream_compatible.cmxa", "prog.ml", "-o", exe].map(String::from));
    let c = run(&cmd, dir, COMPILE_TIMEOUT);
    if c.code != 0 {
        let (kind, area, key) = classify_crash(&c);
        let msg = format!("exit {}: {}", c.code, c.stderr.lines().take(6).collect::<Vec<_>>().join(" | "));
        return Check::Crash { kind, area, key, msg };
    }
    let n = run(&[format!("./{exe}")], dir, RUN_TIMEOUT);
    if n.code == r.code && n.stdout == r.stdout {
        return Check::Same;
    }
    let r2 = run(&["./p.byte".into()], dir, RUN_TIMEOUT * 2);
    let n2 = run(&[format!("./{exe}")], dir, RUN_TIMEOUT * 2);
    if r2.code != r.code || r2.stdout != r.stdout || n2.code != n.code || n2.stdout != n.stdout {
        cx.bump("flaky_diff");
        return Check::Same;
    }
    let kind = diff_kind(&r, &n);
    let area = diff_area(&r.stdout, &n.stdout, foci);
    let msg = format!("exit {} vs bytecode {}; first diff: {}", n.code, r.code, first_diff(&r.stdout, &n.stdout));
    Check::Diff { kind, area, msg }
}

/// Toggle oracle: one more native compile with a base level plus 1-3 switches
/// from TOGGLES. On a failure, localise: if the base level alone fails the same
/// way it is an ordinary diff/crash; otherwise the first single switch that
/// reproduces names the signature (`toggle__<kind>__<switch>` for wrong code,
/// `crash__<kind>__<area>_with_<switch>` for compiler crashes, recorded with
/// the minimal flags).
fn toggle_case(cx: &mut Ctx, dir: &Path, seed: u64, prog: &str, r: &Out, foci: &[gen::Focus], sigs: &mut HashSet<String>) -> bool {
    let mut h = splitmix(seed ^ 0x7a6f_6767_6c65);
    let base = TOGGLE_BASES[(h % TOGGLE_BASES.len() as u64) as usize];
    h = splitmix(h);
    let k = 1 + (h % 3) as usize;
    let mut picked: Vec<usize> = Vec::new();
    while picked.len() < k {
        h = splitmix(h);
        let i = (h % TOGGLES.len() as u64) as usize;
        // switches with known, frequently-firing crashes are drawn 32x less often so
        // they do not crowd out the rest (and keep the clean failure rate low)
        if RARE_TOGGLES.contains(&TOGGLES[i].0) && (h >> 40) % 32 != 0 {
            continue;
        }
        if !picked.contains(&i) {
            picked.push(i);
        }
    }
    let num = (splitmix(h) % 64 + 1).to_string();
    let tf = |i: usize| TOGGLES[i].1.replace("{s}", &num);
    let join = |parts: &[String]| parts.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join(" ");
    let mut all = vec![base.to_string()];
    all.extend(picked.iter().map(|&i| tf(i)));
    let flags = join(&all);
    cx.bump("toggle_compiles");
    let c = check_native(cx, dir, &flags, "p_tog.exe", r, foci);
    let Some(full_sig) = c.sig("toggle") else { return false };
    // localise
    let (sig, rec_flags, chk, name) = if check_native(cx, dir, base, "p_togb.exe", r, foci).sig("toggle") == Some(full_sig.clone()) {
        let s = match &c {
            Check::Diff { kind, area, .. } => format!("diff__{kind}__{area}"),
            _ => full_sig.clone(),
        };
        (s, base.to_string(), c, "base level".to_string())
    } else {
        let mut found = None;
        for &i in &picked {
            let f1 = join(&[base.to_string(), tf(i)]);
            let c1 = check_native(cx, dir, &f1, "p_tog1.exe", r, foci);
            if let Some(s1) = c1.sig("toggle") {
                found = Some((i, f1, c1, s1));
                break;
            }
        }
        match found {
            Some((i, f1, c1, s1)) => {
                // a crash only this switch provokes gets the switch in its area, so
                // it does not hide behind a known crash with the same message
                let s = match &c1 {
                    Check::Diff { kind, .. } => format!("toggle__{kind}__{}", TOGGLES[i].0),
                    _ => format!("{s1}_with_{}", TOGGLES[i].0),
                };
                (s, f1, c1, TOGGLES[i].0.to_string())
            }
            None => {
                let s = match &c {
                    Check::Diff { kind, .. } => format!("toggle__{kind}__combo"),
                    _ => full_sig.clone(),
                };
                (s, flags.clone(), c, "combination".to_string())
            }
        }
    };
    if !sigs.insert(sig.clone()) {
        return false;
    }
    match chk {
        Check::Crash { kind, key, msg, .. } => {
            let m = format!("native compile failed with toggle ({rec_flags}; culprit: {name}), {msg}");
            cx.record(&sig, "crash", &m, seed, prog, &rec_flags, ReproKind::Crash { key, timeout: kind == "timeout" });
        }
        Check::Diff { msg, .. } => {
            let m = format!("toggle ({rec_flags}; culprit: {name}) differs from bytecode: {msg}");
            cx.record(&sig, "toggle", &m, seed, prog, &rec_flags, ReproKind::Diff);
        }
        Check::Same => return false,
    }
    true
}

fn first_diff(a: &str, b: &str) -> String {
    let al: Vec<&str> = a.lines().collect();
    let bl: Vec<&str> = b.lines().collect();
    for i in 0..al.len().max(bl.len()) {
        if al.get(i) != bl.get(i) {
            return format!("line {}: bytecode {:?} native {:?}", i + 1, al.get(i).unwrap_or(&"<none>"), bl.get(i).unwrap_or(&"<none>"));
        }
    }
    "none".into()
}

fn merge_coverage(cx: &mut Ctx) {
    // bisect_ppx writes one file per compiler process; merge them in place
    let (Ok(bf), Ok(rep)) = (std::env::var("BISECT_FILE"), std::env::var("OXFUZZ_BISECT_REPORT")) else { return };
    let p = PathBuf::from(&bf);
    let Some(dir) = p.parent() else { return };
    let Ok(dir) = fs::canonicalize(if dir.as_os_str().is_empty() { Path::new(".") } else { dir }) else { return };
    let dir = dir.as_path();
    let files: Vec<PathBuf> = fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "coverage").unwrap_or(false)).collect())
        .unwrap_or_default();
    if files.len() < 50 {
        return;
    }
    let target = dir.join(format!("merged-{}-{}.coverage", std::process::id(), cx.cases));
    let mut cmd = Command::new(&rep);
    // cwd "/": bisect-ppx-report rewrites absolute source paths under its cwd
    // to relative ones, which a report run from another directory cannot find
    cmd.arg("merge").arg(&target).args(&files).current_dir("/");
    if let Ok(o) = cmd.output() {
        if o.status.success() {
            for f in &files {
                let _ = fs::remove_file(f);
            }
            cx.bump("coverage_merges");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("gen") => {
            let seed: u64 = args[2].parse().unwrap();
            let p = generate(seed);
            print!("{}", p.text);
            eprintln!("foci: {:?}", p.foci.iter().map(|f| f.name()).collect::<Vec<_>>());
        }
        Some("classify") => {
            // oxfuzz classify <stderr-file> <exit-code>
            let o = Out { code: args[3].parse().unwrap(), stdout: String::new(), stderr: fs::read_to_string(&args[2]).unwrap() };
            let (k, a, key) = classify_crash(&o);
            println!("crash__{k}__{a}\nkey: {key}");
        }
        Some("repro-script") => {
            // oxfuzz repro-script <sig> <flags> diff|crash [grep-key]
            let kind = if args[4] == "diff" { ReproKind::Diff } else { ReproKind::Crash { key: args.get(5).cloned().unwrap_or_default(), timeout: false } };
            print!("{}", repro_script(&args[2], &args[3], &kind));
        }
        Some("run") => {
            let build = PathBuf::from(&args[2]);
            let seed: u64 = args[3].parse().unwrap();
            let secs: u64 = args[4].parse().unwrap();
            let out = PathBuf::from(&args[5]);
            let _ = fs::create_dir_all(out.join("findings"));
            let bin = build.join("install/bin");
            let ocamlopt = std::env::var("OXFUZZ_OCAMLOPT").unwrap_or_else(|_| bin.join("ocamlopt.opt").to_string_lossy().into_owned());
            let work = std::env::var("OXFUZZ_WORK").map(PathBuf::from).unwrap_or_else(|_| out.join("work"));
            let mut cx = Ctx { bin, ocamlopt, out: out.clone(), work, cases: 0, failures: 0, extra: BTreeMap::new(), start: Instant::now() };
            cx.write_stats();
            let deadline = Instant::now() + Duration::from_secs(secs);
            let mut i = 0u64;
            while Instant::now() < deadline {
                one_case(&mut cx, seed, i);
                i += 1;
                cx.write_stats();
                if i % 25 == 0 {
                    merge_coverage(&mut cx);
                }
            }
            let _ = fs::remove_dir_all(&cx.work);
            cx.write_stats();
            let _ = std::io::stdout().flush();
        }
        _ => {
            eprintln!("usage: oxfuzz gen <seed> | oxfuzz run <build> <seed> <secs> <out>");
            std::process::exit(2);
        }
    }
}
