// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! ocfuzz: ephyra-based differential + crash harness for the OCaml native compiler.
//!
//! Oracles (README.md):
//! - diff: bytecode (ocamlc + ocamlrund) vs ocamlopt in several closure and
//!   flambda configurations must print the same output and exit the same way
//!   (manual/src/cmds/native.etex: "the two compilers should generate code that
//!   behave identically").
//! - ref: trunk bytecode vs the OCaml 5.4.0 release bytecode (ref54.sh): sees
//!   front-end / Lambda bugs that trunk bytecode and native share.
//! - crash: a compiler that accepts the program in bytecode must not fail with
//!   a fatal error / assertion / uncaught exception in native, and no binary
//!   may die from a signal or a runtime assertion.
//!
//! Each case is one test of `ephyra::check`: the program is drawn from the
//! test's `Source` (gen.rs), and a failure is shrunk by ephyra through
//! `CaseGen::shrink` (delete tests, items of A, lines of tests). While a check
//! is shrinking, the property only re-evaluates the failing config and accepts
//! a candidate only if it fails with the same check kind and config.
mod gen;

use std::collections::HashMap;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const H_ML: &str = include_str!("h.ml");
const COMPAT54_ML: &str = include_str!("compat54.ml");

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Byte,
    Native,
    /// OCaml 5.4.0 release bytecode (`<build>/ref54/inst`), the ref oracle's reference
    Ref,
}

#[derive(Clone, Debug)]
struct Cfg {
    name: &'static str,
    build: &'static str, // closure | flambda | ref54
    kind: Kind,
    flags: &'static [&'static str],
    /// meta oracle: compiles A re-wrapped as drawn in `Case::wrap` (meta_a)
    meta: bool,
}

fn configs() -> Vec<Cfg> {
    vec![
        Cfg { name: "byte", build: "closure", kind: Kind::Byte, flags: &[], meta: false },
        Cfg { name: "closure", build: "closure", kind: Kind::Native, flags: &[], meta: false },
        Cfg { name: "closure_inline", build: "closure", kind: Kind::Native, flags: &["-inline", "200", "-dcmm-invariants", "-g"], meta: false },
        // register allocator / code-size / scheduling toggles must not change results.
        // closure_linscan replaces closure_compact on 1 case in LINSCAN_EVERY (drawn by
        // CaseGen): -linscan miscompiled atomics on ~20% of cases in run 2 (#15189, now
        // in patches/); kept a minority config in case a sibling bug shows up as often.
        Cfg { name: "closure_compact", build: "closure", kind: Kind::Native, flags: &["-compact", "-no-float-const-prop", "-no-insn-sched", "-function-sections"], meta: false },
        Cfg { name: "closure_linscan", build: "closure", kind: Kind::Native, flags: &["-linscan", "-compact", "-no-float-const-prop", "-no-insn-sched", "-function-sections"], meta: false },
        Cfg { name: "flambda_o3", build: "flambda", kind: Kind::Native, flags: &["-O3"], meta: false },
        Cfg { name: "flambda_classic", build: "flambda", kind: Kind::Native, flags: &["-Oclassic", "-g"], meta: false },
        Cfg {
            name: "flambda_o3x",
            build: "flambda",
            kind: Kind::Native,
            flags: &["-O3", "-unbox-closures", "-rounds", "4", "-inline-max-depth", "4", "-remove-unused-arguments", "-runtime-variant", "d"],
            meta: false,
        },
        Cfg { name: "ref54", build: "ref54", kind: Kind::Ref, flags: &[], meta: false },
        // meta oracle: the same program with A re-wrapped (meta_a) must print what
        // the original prints in bytecode
        Cfg { name: "meta_byte", build: "closure", kind: Kind::Byte, flags: &[], meta: true },
        Cfg { name: "meta_closure", build: "closure", kind: Kind::Native, flags: &[], meta: true },
        Cfg { name: "meta_o3", build: "flambda", kind: Kind::Native, flags: &["-O3"], meta: true },
    ]
}

/// OCAMLRUNPARAM of a config's program. The debug runtimes print a banner on
/// stderr unless verbosity is 0. Two native configs run with a small minor
/// heap (32k words instead of 256k), so that minor collections, which move
/// young blocks, happen inside the short generated tests: a value the native
/// code does not register as a GC root is then read stale. Heap sizes are not
/// observable by the generated programs (no Gc stats, finalisers or weak values).
fn runparam(c: &Cfg) -> &'static str {
    match c.name {
        "closure_inline" | "flambda_o3x" => "v=0,s=32k",
        _ => "v=0",
    }
}

/// One case in LINSCAN_EVERY runs closure_linscan instead of closure_compact.
const LINSCAN_EVERY: u32 = 4;

// wall clock under the gate load (4 searches x 3 workers x 7 compiles on 16 cpus): heavy but
// terminating flambda -rounds 4 compiles took 27 s alone and > 60 s loaded
const COMPILE_TIMEOUT: u64 = 150;
const RUN_TIMEOUT: u64 = 10;
/// tests per `ephyra::check` call (bounded so every check reports inside a burn)
const TESTS_PER_CHECK: u64 = 40;

// ------------------------------------------------------------------ cases

/// One generated test program plus the config choice drawn with it.
#[derive(Clone, PartialEq)]
struct Case {
    prog: gen::Program,
    linscan: bool,
    /// meta oracle: index in WRAPS of how A is re-wrapped
    wrap: usize,
}

impl fmt::Debug for Case {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ids: Vec<usize> = self.prog.tests.iter().map(|t| t.0).collect();
        let lines: usize = self.prog.tests.iter().map(|t| t.1.lines().count()).sum();
        write!(f, "Case {{ tests: {ids:?}, a_lines: {}, main_lines: {lines}, linscan: {}, wrap: {} }}", self.prog.a.lines().count(), self.linscan, WRAPS[self.wrap])
    }
}

/// Top-level items of A: a new item starts at every column-0 line that begins a
/// definition (comments and continuation lines stay with the previous item).
fn a_items(a: &str) -> Vec<String> {
    let mut items: Vec<String> = vec![];
    for line in a.lines() {
        let starts = ["let", "module", "class", "type", "exception", "open"].iter().any(|k| line.starts_with(k));
        if starts || items.is_empty() {
            items.push(String::new());
        }
        let it = items.last_mut().unwrap();
        it.push_str(line);
        it.push('\n');
    }
    items
}

struct CaseGen;

impl ephyra::Gen for CaseGen {
    type Value = Case;

    fn generate(&self, g: &mut ephyra::Source) -> Case {
        let linscan = g.int(0..LINSCAN_EVERY) == 0;
        let wrap = g.index(WRAPS.len());
        let prog = gen::Gen::new(g).program();
        Case { prog, linscan, wrap }
    }

    /// Deletions only (strictly smaller): keep one test; drop half the tests;
    /// drop one test; drop one top-level item of A; drop one line of a test
    /// (never its header or final `()`). Candidates that no longer type-check
    /// do not reproduce (the property passes them).
    fn shrink(&self, v: &Case, visit: &mut dyn FnMut(Case) -> ControlFlow<()>) -> ControlFlow<()> {
        let with_tests = |tests: Vec<(usize, String)>| Case { prog: gen::Program { a: v.prog.a.clone(), tests }, linscan: v.linscan, wrap: v.wrap };
        let ts = &v.prog.tests;
        if ts.len() > 1 {
            for t in ts {
                visit(with_tests(vec![t.clone()]))?;
            }
            if ts.len() > 3 {
                let h = ts.len() / 2;
                visit(with_tests(ts[..h].to_vec()))?;
                visit(with_tests(ts[h..].to_vec()))?;
            }
            for i in 0..ts.len() {
                let mut t2 = ts.clone();
                t2.remove(i);
                visit(with_tests(t2))?;
            }
        }
        let items = a_items(&v.prog.a);
        for i in (0..items.len()).rev() {
            let a: String = items.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, s)| s.as_str()).collect();
            visit(Case { prog: gen::Program { a, tests: ts.clone() }, linscan: v.linscan, wrap: v.wrap })?;
        }
        for (ti, (_, body)) in ts.iter().enumerate() {
            let lines: Vec<&str> = body.lines().collect();
            for li in (1..lines.len().saturating_sub(1)).rev() {
                let mut nb = String::new();
                for (k, l) in lines.iter().enumerate() {
                    if k != li {
                        nb.push_str(l);
                        nb.push('\n');
                    }
                }
                let mut t2 = ts.clone();
                t2[ti].1 = nb;
                visit(with_tests(t2))?;
            }
        }
        ControlFlow::Continue(())
    }
}

// ---------------------------------------------------------------- running

struct Out {
    status: Option<i32>, // None: timeout
    signal: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_cmd(cmd: &mut Command, timeout: u64) -> Out {
    use std::os::unix::process::ExitStatusExt;
    let mut child = match cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => return Out { status: Some(127), signal: None, stdout: String::new(), stderr: format!("spawn: {e}") },
    };
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let t1 = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = so.read_to_end(&mut v);
        v
    });
    let t2 = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = se.read_to_end(&mut v);
        v
    });
    let start = Instant::now();
    let st = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if start.elapsed() > Duration::from_secs(timeout) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break None,
        }
    };
    let stdout = String::from_utf8_lossy(&t1.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&t2.join().unwrap_or_default()).into_owned();
    match st {
        None => Out { status: None, signal: None, stdout, stderr },
        // killed by a signal: status 128+sig (status None means timeout)
        Some(st) => Out { status: Some(st.code().unwrap_or(128 + st.signal().unwrap_or(0))), signal: st.signal(), stdout, stderr },
    }
}

enum Res {
    CompileFail(String),
    CompileTimeout,
    Ran(Out),
}

fn compile_and_run(build: &Path, dir: &Path, c: &Cfg) -> Res {
    let d = dir.join(c.name);
    let _ = fs::remove_dir_all(&d);
    let _ = fs::create_dir_all(&d);
    for f in ["h.ml", "a.ml", "main.ml"] {
        let _ = fs::copy(dir.join(f), d.join(f));
    }
    let bin = build.join(c.build).join("inst/bin");
    if c.meta {
        let wrap = fs::read_to_string(dir.join("wrap")).unwrap_or_default();
        let a = fs::read_to_string(dir.join("a.ml")).unwrap_or_default();
        match meta_a(build, &d, &a, wrap.trim()) {
            Ok(m) => {
                let _ = fs::write(d.join("a.ml"), m);
            }
            Err(e) => return Res::CompileFail(format!("meta: {e}")),
        }
        let main = fs::read_to_string(dir.join("main.ml")).unwrap_or_default();
        let _ = fs::write(d.join("main.ml"), meta_main(&main, wrap.trim()));
    }
    let (comp, exe) = match c.kind {
        Kind::Byte | Kind::Ref => ("ocamlc", "p.byte"),
        Kind::Native => ("ocamlopt", "p.exe"),
    };
    let mut cmd = Command::new(bin.join(comp));
    if c.kind == Kind::Ref {
        // 5.4.0 lacks some trunk stdlib functions: compat54.ml re-implements them
        let _ = fs::write(d.join("compat54.ml"), COMPAT54_ML);
        let o = run_cmd(Command::new(bin.join(comp)).current_dir(&d).args(["-w", "-a", "-c", "compat54.ml"]), COMPILE_TIMEOUT);
        if o.status != Some(0) {
            return Res::CompileFail(format!("{}{}", o.stdout, o.stderr));
        }
        cmd.args(["-open", "Compat54", "compat54.cmo"]);
    }
    cmd.current_dir(&d).args(["-w", "-a"]).args(c.flags).args(["h.ml", "a.ml", "main.ml", "-o", exe]);
    let o = run_cmd(&mut cmd, COMPILE_TIMEOUT);
    match o.status {
        None => return Res::CompileTimeout,
        Some(0) => {}
        _ => return Res::CompileFail(format!("{}{}", o.stdout, o.stderr)),
    }
    let mut cmd = match c.kind {
        Kind::Byte => {
            let mut cmd = Command::new(bin.join("ocamlrund"));
            cmd.arg(exe);
            cmd
        }
        Kind::Ref => {
            let mut cmd = Command::new(bin.join("ocamlrun"));
            cmd.arg(exe);
            cmd
        }
        Kind::Native => Command::new(d.join(exe)),
    };
    cmd.current_dir(&d).env("OCAMLRUNPARAM", runparam(c));
    Res::Ran(run_cmd(&mut cmd, RUN_TIMEOUT))
}

/// meta oracle: ways of re-wrapping module A that must not change what the
/// program prints. `fun`: A's body in a generative functor applied once
/// (`include Body ()`: A's top level becomes a function body, translmod's
/// functor path, closure/flambda functor inlining, no static allocation of A's
/// globals); `rec`: A as before plus a recursive module whose signature has
/// A's function-typed values (a safe module: CamlinternalMod init_mod /
/// update_mod, translmod compile_recmodule), and A's functions rebound to the
/// recursive module's; `fcm`: A packed as a first-class module of its own
/// signature (`ocamlc -i`) and unpacked with `include (val ...)`; `main`: A
/// unchanged, Main's body (the test functions and the toplevel `let () =`
/// that run them) in a generative functor applied once, so the test functions
/// become local closures of the functor body instead of static ones.
const WRAPS: &[&str] = &["fun", "rec", "fcm", "main"];

/// Main's source for wrap `wrap` (only `main` changes it).
fn meta_main(main: &str, wrap: &str) -> String {
    if wrap == "main" {
        format!("module Body () = struct\n# 1 \"main.ml\"\n{main}\nend\nmodule _ = Body ()\n")
    } else {
        main.to_string()
    }
}

/// Top-level items of an `ocamlc -i` signature (continuation lines are indented).
fn sig_items(sig: &str) -> Vec<String> {
    let mut items: Vec<String> = vec![];
    for line in sig.lines() {
        if !line.starts_with(' ') || items.is_empty() {
            items.push(String::new());
        }
        let it = items.last_mut().unwrap();
        it.push_str(line);
        it.push('\n');
    }
    items
}

/// Name of a `val` item whose type is a function (a value kind a safe
/// recursive module may contain, translmod.ml init_shape). Lazy values are
/// safe too but left out: update_mod copies an unforced lazy, and forcing the
/// copy and the original would run its body twice.
fn safe_val(item: &str) -> Option<String> {
    let rest = item.strip_prefix("val ")?;
    let (name, ty) = rest.split_once(" : ")?;
    let ty = ty.trim();
    let mut depth = 0i32;
    let b = ty.as_bytes();
    let mut arrow = false;
    for i in 0..b.len() {
        match b[i] {
            b'(' | b'[' | b'<' => depth += 1,
            b')' | b']' => depth -= 1,
            b'>' if i > 0 && b[i - 1] == b'-' => arrow |= depth == 0,
            b'>' => depth -= 1,
            _ => {}
        }
    }
    if arrow && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some(name.to_string())
    } else {
        None
    }
}

/// Line directive in front of A's original text: Match_failure and
/// Assert_failure carry source locations, which must not move.
const LINE1: &str = "# 1 \"a.ml\"\n";

/// A's source re-wrapped as `wrap` (WRAPS). `d` holds h.ml and a.ml; the
/// signature is inferred with the closure build's ocamlc.
fn meta_a(build: &Path, d: &Path, a: &str, wrap: &str) -> Result<String, String> {
    if wrap == "main" {
        return Ok(a.to_string());
    }
    if wrap == "fun" {
        return Ok(format!("module Body () = struct\n{LINE1}{a}\nend\ninclude Body ()\n"));
    }
    let oc = build.join("closure/inst/bin/ocamlc");
    let o = run_cmd(Command::new(&oc).current_dir(d).args(["-w", "-a", "-c", "h.ml"]), COMPILE_TIMEOUT);
    if o.status != Some(0) {
        return Err(format!("h.ml: {}{}", o.stdout, o.stderr));
    }
    let o = run_cmd(Command::new(&oc).current_dir(d).args(["-w", "-a", "-i", "a.ml"]), COMPILE_TIMEOUT);
    if o.status != Some(0) {
        return Err(format!("-i: {}{}", o.stdout, o.stderr));
    }
    for f in ["h.cmi", "h.cmo", "a.cmi", "a.cmo"] {
        let _ = fs::remove_file(d.join(f));
    }
    let sig = o.stdout;
    match wrap {
        "rec" => {
            let fns: Vec<(String, String)> = sig_items(&sig).into_iter().filter_map(|it| safe_val(&it).map(|n| (n, it))).collect();
            let mut s = format!("module Body0 = struct\n{LINE1}{a}\nend\ninclude Body0\nmodule rec Body : sig\n");
            for (_, it) in &fns {
                s.push_str(it);
            }
            s.push_str("end = struct include Body0 end\n");
            for (n, _) in &fns {
                let _ = writeln!(s, "let {n} = Body.{n}");
            }
            Ok(s)
        }
        _ => Ok(format!("module type SIG = sig\n{sig}end\ninclude (val (module struct\n{LINE1}{a}\nend : SIG))\n")),
    }
}

/// Compiler failure that is a crash (not an ordinary type error).
fn is_compiler_crash(msg: &str) -> bool {
    msg.contains("Fatal error")
        || msg.contains("Assert_failure")
        || msg.contains("Uncaught exception")
        || msg.contains("Stack_overflow")
        || msg.contains("Out of memory")
        || msg.contains("Segmentation")
        || msg.contains(">> Fatal")
        || msg.contains("Error during linking")
        || msg.contains("Assembler error")
}

/// Stable area for a compiler crash: the compiler source file named in the
/// assertion/exception if any, otherwise "other".
fn crash_area(msg: &str) -> String {
    if let Some(i) = msg.find("Assert_failure(\"") {
        let rest = &msg[i + 16..];
        if let Some(j) = rest.find('"') {
            let f = &rest[..j];
            let base = f.rsplit('/').next().unwrap_or(f);
            return sanitize(base.trim_end_matches(".ml"));
        }
    }
    if msg.contains("Assembler error") || msg.contains("Error during linking") {
        return "link".into();
    }
    if msg.contains("invariant") {
        return "flambda_invariant".into();
    }
    if let Some(i) = msg.find(">> Fatal error: ") {
        let w: String = msg[i + 16..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
        let w = sanitize(&w.to_lowercase());
        if !w.is_empty() {
            return w;
        }
    }
    if msg.contains("Misc.Fatal_error") || msg.contains("Fatal error") {
        return "fatal_error".into();
    }
    if msg.contains("Stack_overflow") {
        return "stack_overflow".into();
    }
    "other".into()
}

/// Stable area for a runtime crash: the runtime source file of a debug-runtime
/// assertion ("file runtime/interp.c; line N ### Assertion failed"), else
/// "fatal_error", else the signal ("sig11"), else "other".
fn run_crash_area(o: &Out) -> String {
    if let Some(i) = o.stderr.find("file runtime/") {
        let f: String = o.stderr[i + 13..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if !f.is_empty() {
            return sanitize(&f);
        }
    }
    if o.stderr.contains("Fatal error") {
        return "fatal_error".into();
    }
    match o.signal {
        Some(n) => format!("sig{n}"),
        None => "other".into(),
    }
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

fn short(cfg: &str) -> &'static str {
    match cfg {
        "closure" => "c",
        "closure_inline" => "ci",
        "closure_compact" => "cc",
        "closure_linscan" => "cl",
        "flambda_o3" => "o3",
        "flambda_classic" => "oc",
        "flambda_o3x" => "ox",
        "byte" | "meta_byte" => "b",
        "meta_closure" => "c",
        "meta_o3" => "o3",
        _ => "x",
    }
}

fn outcome(o: &Out) -> String {
    format!("{}\n--exit {:?} signal {:?}", o.stdout, o.status, o.signal)
}

/// First test section ("==tN") where the two outputs differ.
fn first_diff_test(a: &str, b: &str) -> Option<usize> {
    let sa: Vec<&str> = a.split("\n==").collect();
    let sb: Vec<&str> = b.split("\n==").collect();
    for i in 1..sa.len().max(sb.len()) {
        let x = sa.get(i).copied().unwrap_or("");
        let y = sb.get(i).copied().unwrap_or("");
        if x != y {
            let name = if !x.is_empty() { x } else { y };
            let n: String = name.trim_start_matches('t').chars().take_while(|c| c.is_ascii_digit()).collect();
            return n.parse().ok();
        }
    }
    None
}

#[derive(Default)]
struct Stats {
    cases: u64,
    failures: u64,
    gen_invalid: u64,
    ref_timeout: u64,
    ref_exn_cases: u64,
    /// 5.4.0 rejected / crashed on / timed out on a program trunk ran: ref oracle skipped
    ref54_skip: u64,
    /// the re-wrapped program did not type-check (or ocamlc -i failed): meta oracle skipped
    meta_skip: u64,
    checks: u64,
    ephyra_tests: u64,
    shrink_evals: u64,
}

#[derive(Clone, Debug)]
struct Finding {
    sig: String,
    message: String,
    cfg: Cfg,
    check: Check,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Check {
    CompileCrash,
    CompileTimeout,
    RunSignal,
    RunTimeout,
    Diff,
    /// ocamlc crashed (compiler) or the bytecode program crashed
    ByteCompileCrash,
    ByteCompileTimeout,
    ByteRunCrash,
    /// trunk bytecode output differs from OCaml 5.4.0 bytecode
    RefDiff,
    /// the program with A re-wrapped (meta cfg) differs from the original's bytecode output
    MetaDiff,
}

enum Eval {
    /// ocamlc rejected the program (type error): a generator bug when fresh
    Invalid(String),
    /// the bytecode run timed out: no reference output
    RefTimeout,
    Done { findings: Vec<Finding>, exn: bool, ref54_skip: bool, meta_skip: bool },
}

/// Runs the configs over the program in `dir` (cfgs[0] is bytecode, the
/// reference). With `sequential`, bytecode is compiled first and the native
/// configs only if it succeeds (cheap rejection while shrinking).
fn evaluate(build: &Path, dir: &Path, cfgs: &[Cfg], sequential: bool) -> Eval {
    let results: Vec<Res> = if sequential {
        let r0 = compile_and_run(build, dir, &cfgs[0]);
        if !matches!(r0, Res::Ran(_)) {
            vec![r0]
        } else {
            let mut v = vec![r0];
            v.extend(cfgs[1..].iter().map(|c| compile_and_run(build, dir, c)));
            v
        }
    } else {
        std::thread::scope(|sc| {
            let hs: Vec<_> = cfgs.iter().map(|c| sc.spawn(move || compile_and_run(build, dir, c))).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        })
    };
    let mut fs_ = vec![];
    let ref_out = match &results[0] {
        Res::CompileFail(m) => {
            if is_compiler_crash(m) {
                fs_.push(Finding {
                    sig: format!("crash__compile__byte_{}", crash_area(m)),
                    message: first_lines(m, 20),
                    cfg: cfgs[0].clone(),
                    check: Check::ByteCompileCrash,
                });
                return Eval::Done { findings: fs_, exn: false, ref54_skip: false, meta_skip: false };
            }
            return Eval::Invalid(m.clone());
        }
        Res::CompileTimeout => {
            fs_.push(Finding { sig: "hang__compile__byte".into(), message: "ocamlc timeout".into(), cfg: cfgs[0].clone(), check: Check::ByteCompileTimeout });
            return Eval::Done { findings: fs_, exn: false, ref54_skip: false, meta_skip: false };
        }
        Res::Ran(o) => {
            if o.status.is_none() {
                return Eval::RefTimeout;
            }
            if o.signal.is_some() || o.stderr.contains("Assertion") || o.stderr.contains("Fatal error") {
                fs_.push(Finding {
                    sig: format!("crash__run__byte_{}", run_crash_area(o)),
                    message: first_lines(&o.stderr, 20),
                    cfg: cfgs[0].clone(),
                    check: Check::ByteRunCrash,
                });
                return Eval::Done { findings: fs_, exn: false, ref54_skip: false, meta_skip: false };
            }
            outcome(o)
        }
    };
    let exn = ref_out.contains("!exn");
    let mut ref54_skip = false;
    let mut meta_skip = false;
    let wrap = fs::read_to_string(dir.join("wrap")).unwrap_or_default();
    // trunk outputs that ran cleanly, for the ref oracle's area
    let mut outs: Vec<(&Cfg, String)> = vec![(&cfgs[0], ref_out.clone())];
    for (c, r) in cfgs.iter().zip(&results).skip(1) {
        let g = c.build;
        if c.kind == Kind::Ref {
            // ref oracle (below): a 5.4.0 compile error (trunk-only feature), crash or
            // timeout is the reference's problem, not the target's: skip the oracle
            match r {
                Res::Ran(o) if o.status.is_some() && o.signal.is_none() => {}
                _ => ref54_skip = true,
            }
            continue;
        }
        if c.meta {
            // the re-wrapped program must compile wherever the original did, but a
            // rejection is only a finding when it is a crash (a type error means
            // the wrapping was not valid OCaml for this A: skip the meta oracle)
            if let Res::CompileFail(m) = r {
                if m.starts_with("meta:") || !is_compiler_crash(m) {
                    meta_skip = true;
                    continue;
                }
            }
            if let Res::Ran(o) = r {
                if o.status.is_some() && o.signal.is_none() && !o.stderr.contains("Assertion") && !o.stderr.contains("Fatal error") {
                    let out = outcome(o);
                    if out != ref_out {
                        let t = first_diff_test(&ref_out, &out);
                        let (a, b) = (section(&ref_out, t), section(&out, t));
                        let kind = if a.contains("!exn") || b.contains("!exn") { "exn" } else { "value" };
                        fs_.push(Finding {
                            sig: format!("meta__{kind}__{}", c.name),
                            message: format!("{} (A wrapped: {}) differs from the original program's bytecode output in test {:?}\nbyte: {}\nmeta: {}", c.name, wrap.trim(), t, first_lines(&a, 6), first_lines(&b, 6)),
                            cfg: c.clone(),
                            check: Check::MetaDiff,
                        });
                    }
                    continue;
                }
            }
        }
        match r {
            Res::CompileFail(m) => {
                // bytecode accepted the program: any native compile failure is a bug
                fs_.push(Finding {
                    sig: format!("crash__compile__{}_{}", g, crash_area(m)),
                    message: first_lines(m, 20),
                    cfg: c.clone(),
                    check: Check::CompileCrash,
                });
            }
            Res::CompileTimeout => {
                fs_.push(Finding { sig: format!("hang__compile__{g}"), message: "ocamlopt timeout".into(), cfg: c.clone(), check: Check::CompileTimeout });
            }
            Res::Ran(o) => {
                if o.status.is_none() {
                    fs_.push(Finding { sig: format!("hang__run__{g}"), message: "native binary timeout".into(), cfg: c.clone(), check: Check::RunTimeout });
                } else if o.signal.is_some() || o.stderr.contains("Assertion") || o.stderr.contains("Fatal error") {
                    fs_.push(Finding {
                        sig: format!("crash__run__{g}_{}", run_crash_area(o)),
                        message: format!("signal {:?}: {}", o.signal, first_lines(&o.stderr, 10)),
                        cfg: c.clone(),
                        check: Check::RunSignal,
                    });
                } else {
                    let out = outcome(o);
                    outs.push((c, out.clone()));
                    if out != ref_out {
                        let t = first_diff_test(&ref_out, &out);
                        let sec = |s: &str| -> String {
                            match t {
                                Some(t) => s.split("\n==").find(|x| x.starts_with(&format!("t{t}\n"))).unwrap_or("").to_string(),
                                None => s.chars().rev().take(300).collect::<String>().chars().rev().collect(),
                            }
                        };
                        let kind = if sec(&ref_out).contains("!exn") || sec(&out).contains("!exn") { "exn" } else { "value" };
                        fs_.push(Finding {
                            sig: format!("diff__{kind}__{g}"),
                            message: format!("{} differs from bytecode in test {:?}\nbyte:   {}\nnative: {}", c.name, t, first_lines(&sec(&ref_out), 6), first_lines(&sec(&out), 6)),
                            cfg: c.clone(),
                            check: Check::Diff,
                        });
                    }
                }
            }
        }
    }
    // Diff signatures: area = the set of native configs that disagree with
    // bytecode (e.g. `c_oc` = closure + flambda_classic): different bugs tend
    // to hit different config sets, so one frequent bug does not hide others.
    // One diff finding per case (the first disagreeing config).
    // meta signatures: area = how A was wrapped + the meta configs that differ
    // from the original bytecode output (`b` meta_byte, `c` meta_closure, `o3`)
    let metas: Vec<&Finding> = fs_.iter().filter(|f| f.check == Check::MetaDiff).collect();
    if !metas.is_empty() {
        let tags: Vec<&str> = metas.iter().map(|f| short(f.cfg.name)).collect();
        let kind = if metas.iter().any(|f| f.sig.starts_with("meta__exn")) { "exn" } else { "value" };
        let d = Finding { sig: format!("meta__{kind}__{}_{}", sanitize(wrap.trim()), tags.join("_")), message: metas[0].message.clone(), cfg: metas[0].cfg.clone(), check: Check::MetaDiff };
        fs_.retain(|f| f.check != Check::MetaDiff);
        fs_.push(d);
    }
    let diffs: Vec<&Finding> = fs_.iter().filter(|f| f.check == Check::Diff).collect();
    if !diffs.is_empty() {
        let tags: Vec<&str> = diffs.iter().map(|f| short(f.cfg.name)).collect();
        let kind = if diffs.iter().any(|f| f.sig.starts_with("diff__exn")) { "exn" } else { "value" };
        let mut d = Finding { sig: format!("diff__{kind}__{}", tags.join("_")), message: diffs[0].message.clone(), cfg: diffs[0].cfg.clone(), check: Check::Diff };
        if diffs.len() > 1 {
            d.message.push_str(&format!("\n(also differing: {})", diffs[1..].iter().map(|f| f.cfg.name).collect::<Vec<_>>().join(", ")));
        }
        fs_.retain(|f| f.check != Check::Diff);
        fs_.push(d);
    }
    // ref oracle: trunk bytecode must print what the OCaml 5.4.0 release prints
    // (the language and the stdlib functions used are unchanged since 5.4.0, the
    // trunk-only ones come from compat54.ml). Area: `all` = bytecode and every
    // native config differ from 5.4.0 (shared front end / Lambda), `byte` = only
    // bytecode (a bytecode-only bug, also visible as a diff finding), else
    // the tags of the trunk configs that differ (`b` + native tags).
    if let Some((rc, Res::Ran(o))) = cfgs.iter().zip(&results).find(|(c, _)| c.kind == Kind::Ref) {
        if o.status.is_some() && o.signal.is_none() {
            let r54 = outcome(o);
            if r54 != ref_out {
                let differ: Vec<&str> = outs.iter().filter(|(_, x)| *x != r54).map(|(c, _)| short(c.name)).collect();
                let area = if differ.len() == outs.len() && outs.len() > 1 {
                    "all".to_string()
                } else if differ == ["b"] {
                    "byte".to_string()
                } else {
                    differ.join("_")
                };
                let t = first_diff_test(&r54, &ref_out);
                let sec = |s: &str| -> String {
                    match t {
                        Some(t) => s.split("\n==").find(|x| x.starts_with(&format!("t{t}\n"))).unwrap_or("").to_string(),
                        None => s.chars().rev().take(300).collect::<String>().chars().rev().collect(),
                    }
                };
                let kind = if sec(&ref_out).contains("!exn") || sec(&r54).contains("!exn") { "exn" } else { "value" };
                fs_.push(Finding {
                    sig: format!("ref__{kind}__{area}"),
                    message: format!("trunk bytecode differs from OCaml 5.4.0 bytecode in test {:?}\n5.4.0: {}\ntrunk: {}", t, first_lines(&sec(&r54), 6), first_lines(&sec(&ref_out), 6)),
                    cfg: rc.clone(),
                    check: Check::RefDiff,
                });
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    fs_.retain(|f| seen.insert(f.sig.clone()));
    Eval::Done { findings: fs_, exn, ref54_skip, meta_skip }
}

/// Section of test `t` ("tN\n...") of a program's output, or its tail.
fn section(s: &str, t: Option<usize>) -> String {
    match t {
        Some(t) => s.split("\n==").find(|x| x.starts_with(&format!("t{t}\n"))).unwrap_or("").to_string(),
        None => s.chars().rev().take(300).collect::<String>().chars().rev().collect(),
    }
}

fn first_lines(s: &str, n: usize) -> String {
    let v: Vec<&str> = s.lines().take(n).collect();
    let mut r = v.join("\n");
    if r.len() > 2000 {
        let mut k = 2000;
        while !r.is_char_boundary(k) {
            k -= 1;
        }
        r.truncate(k);
    }
    r
}

fn write_program(dir: &Path, p: &gen::Program, wrap: usize) {
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(dir.join("wrap"), WRAPS[wrap]);
    let _ = fs::write(dir.join("h.ml"), H_ML);
    let _ = fs::write(dir.join("a.ml"), &p.a);
    let _ = fs::write(dir.join("main.ml"), p.main_src());
    let _ = fs::write(dir.join("compat54.ml"), COMPAT54_ML);
}

fn repro_script(f: &Finding) -> String {
    let c = &f.cfg;
    let flags = c.flags.join(" ");
    let rp = runparam(c);
    let comp = if c.kind == Kind::Byte { "ocamlc" } else { "ocamlopt" };
    let exe = if c.kind == Kind::Byte { "p.byte" } else { "p.exe" };
    let runexe = if c.kind == Kind::Byte { format!("\"$B/{}/inst/bin/ocamlrund\" ./p.byte", c.build) } else { "./p.exe".into() };
    let head = format!(
        r#"#!/bin/bash
# Reproduces: {sig}
# usage: repro.sh <build-dir>   (exit 1: reproduces, 0: does not, 2: cannot decide)
B=$(realpath "$1")
ulimit -c 0
export OCAMLRUNPARAM=v=0
cd "$(dirname "$0")"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
cp h.ml a.ml main.ml compat54.ml "$T"/
{meta}cd "$T"
CB="$B/closure/inst/bin"
TB="$B/{build}/inst/bin"
"#,
        sig = f.sig,
        build = c.build,
        // meta config: a_meta.ml/main_meta.ml are A/Main as the config compiled them; a
        // MetaDiff compares it with the original program, other checks use it alone
        meta = match (c.meta, f.check) {
            (false, _) => "",
            (true, Check::MetaDiff) => "cp a_meta.ml main_meta.ml \"$T\"/\n",
            (true, _) => "cp a_meta.ml \"$T\"/a.ml\ncp main_meta.ml \"$T\"/main.ml\n",
        }
    );
    let body = match f.check {
        Check::CompileCrash | Check::ByteCompileCrash => format!(
            r#""$CB/ocamlc" -w -a h.ml a.ml main.ml -o ref.byte > ref.log 2>&1 || {{ [ "{comp}" = ocamlc ] || exit 2; }}
timeout {COMPILE_TIMEOUT} "$TB/{comp}" -w -a {flags} h.ml a.ml main.ml -o {exe} > c.log 2>&1
st=$?
[ $st -eq 124 ] && exit 2
[ $st -eq 0 ] && exit 0
grep -qE 'Fatal error|Assert_failure|Uncaught exception|Stack_overflow|Out of memory|Segmentation|Error during linking|Assembler error' c.log && exit 1
exit 2
"#
        ),
        Check::CompileTimeout | Check::ByteCompileTimeout => format!(
            r#""$CB/ocamlc" -w -a h.ml a.ml main.ml -o ref.byte > ref.log 2>&1 || {{ [ "{comp}" = ocamlc ] || exit 2; }}
timeout {COMPILE_TIMEOUT} "$TB/{comp}" -w -a {flags} h.ml a.ml main.ml -o {exe} > c.log 2>&1
[ $? -eq 124 ] && exit 1
exit 0
"#
        ),
        Check::ByteRunCrash => format!(
            r#""$CB/ocamlc" -w -a h.ml a.ml main.ml -o p.byte > c.log 2>&1 || exit 2
timeout {RUN_TIMEOUT} "$CB/ocamlrund" ./p.byte > out.txt 2> err.txt
st=$?
[ $st -eq 124 ] && exit 2
[ $st -gt 128 ] && exit 1
grep -qE 'Assertion|Fatal error' err.txt && exit 1
exit 0
"#
        ),
        Check::RefDiff => format!(
            r#"RB="$B/ref54/inst/bin"
[ -x "$RB/ocamlc" ] || exit 2
"$RB/ocamlc" -w -a -c compat54.ml > r54.log 2>&1 || exit 2
"$RB/ocamlc" -w -a -open Compat54 compat54.cmo h.ml a.ml main.ml -o r54.byte >> r54.log 2>&1 || exit 2
timeout {RUN_TIMEOUT} "$RB/ocamlrun" ./r54.byte > r54.txt 2> r54.err
rst=$?
[ $rst -eq 124 ] && exit 2
[ $rst -gt 128 ] && exit 2
echo "--exit $rst" >> r54.txt
"$CB/ocamlc" -w -a h.ml a.ml main.ml -o p.byte > c.log 2>&1 || exit 2
timeout {RUN_TIMEOUT} "$CB/ocamlrund" ./p.byte > out.txt 2> err.txt
st=$?
[ $st -eq 124 ] && exit 2
[ $st -gt 128 ] && exit 2
echo "--exit $st" >> out.txt
cmp -s r54.txt out.txt && exit 0
exit 1
"#
        ),
        Check::MetaDiff => format!(
            r#"[ -f a_meta.ml ] && [ -f main_meta.ml ] || exit 2
"$CB/ocamlc" -w -a h.ml a.ml main.ml -o ref.byte > ref.log 2>&1 || exit 2
timeout {RUN_TIMEOUT} "$CB/ocamlrund" ./ref.byte > ref.txt 2> ref.err
rst=$?
[ $rst -eq 124 ] && exit 2
[ $rst -gt 128 ] && exit 2
echo "--exit $rst" >> ref.txt
mkdir m && cp h.ml m/ && cp a_meta.ml m/a.ml && cp main_meta.ml m/main.ml && cd m || exit 2
timeout {COMPILE_TIMEOUT} "$TB/{comp}" -w -a {flags} h.ml a.ml main.ml -o {exe} > c.log 2>&1 || exit 2
OCAMLRUNPARAM={rp} timeout {RUN_TIMEOUT} {runexe} > out.txt 2> err.txt
st=$?
[ $st -eq 124 ] && exit 2
[ $st -gt 128 ] && exit 2
echo "--exit $st" >> out.txt
cmp -s ../ref.txt out.txt && exit 0
exit 1
"#
        ),
        Check::RunSignal | Check::RunTimeout | Check::Diff => {
            let decide = match f.check {
                Check::RunSignal => r#"[ $st -eq 124 ] && exit 2
[ $st -gt 128 ] && exit 1
grep -qE 'Assertion|Fatal error' err.txt && exit 1
exit 0"#
                    .to_string(),
                Check::RunTimeout => "[ $st -eq 124 ] && exit 1\nexit 0".to_string(),
                _ => r#"[ $st -eq 124 ] && exit 2
[ $st -gt 128 ] && exit 2
echo "--exit $st" >> out.txt
cmp -s ref.txt out.txt && exit 0
exit 1"#
                    .to_string(),
            };
            format!(
                r#""$CB/ocamlc" -w -a h.ml a.ml main.ml -o ref.byte > ref.log 2>&1 || exit 2
timeout {RUN_TIMEOUT} "$CB/ocamlrund" ./ref.byte > ref.txt 2> ref.err
rst=$?
[ $rst -eq 124 ] && exit 2
[ $rst -gt 128 ] && exit 2
echo "--exit $rst" >> ref.txt
timeout {COMPILE_TIMEOUT} "$TB/{comp}" -w -a {flags} h.ml a.ml main.ml -o {exe} > c.log 2>&1 || exit 2
OCAMLRUNPARAM={rp} timeout {RUN_TIMEOUT} {runexe} > out.txt 2> err.txt
st=$?
{decide}
"#
            )
        }
    };
    head + &body
}

/// Fails the current ephyra test. One panic site per check kind: ephyra keeps
/// "the same failure" while shrinking by panic location.
fn fail(f: &Finding) -> ! {
    match f.check {
        Check::Diff => panic!("{}: {}", f.sig, f.message),
        Check::CompileCrash => panic!("{}: {}", f.sig, f.message),
        Check::CompileTimeout => panic!("{}: {}", f.sig, f.message),
        Check::RunSignal => panic!("{}: {}", f.sig, f.message),
        Check::RunTimeout => panic!("{}: {}", f.sig, f.message),
        Check::ByteCompileCrash => panic!("{}: {}", f.sig, f.message),
        Check::ByteCompileTimeout => panic!("{}: {}", f.sig, f.message),
        Check::ByteRunCrash => panic!("{}: {}", f.sig, f.message),
        Check::RefDiff => panic!("{}: {}", f.sig, f.message),
        Check::MetaDiff => panic!("{}: {}", f.sig, f.message),
    }
}

fn write_stats(out: &Path, st: &Stats, start: Instant) {
    let el = start.elapsed().as_secs_f64();
    let s = format!(
        "{{\"cases\": {}, \"failures\": {}, \"gen_invalid\": {}, \"ref_timeout\": {}, \"ref_exn_cases\": {}, \"ref54_skip\": {}, \"meta_skip\": {}, \"checks\": {}, \"ephyra_tests\": {}, \"shrink_evals\": {}, \"elapsed_s\": {:.1}, \"cases_per_s\": {:.3}}}\n",
        st.cases,
        st.failures,
        st.gen_invalid,
        st.ref_timeout,
        st.ref_exn_cases,
        st.ref54_skip,
        st.meta_skip,
        st.checks,
        st.ephyra_tests,
        st.shrink_evals,
        el,
        st.cases as f64 / el.max(0.001)
    );
    let tmp = out.join("stats.json.tmp");
    let _ = fs::write(&tmp, s);
    let _ = fs::rename(tmp, out.join("stats.json"));
}

fn arg(args: &[String], k: &str) -> Option<String> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1).cloned())
}

fn case_cfgs(cfgs: &[Cfg], c: &Case) -> Vec<Cfg> {
    let drop = if c.linscan { "closure_compact" } else { "closure_linscan" };
    cfgs.iter().filter(|x| x.name != drop).cloned().collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("gen") => {
            // write the program of ephyra source seed S (debugging / samples)
            let seed: u64 = arg(&args, "--seed").unwrap().parse().unwrap();
            let dir = PathBuf::from(arg(&args, "--dir").unwrap());
            let mut src = ephyra::Source::new(seed, 10);
            let c = <CaseGen as ephyra::Gen>::generate(&CaseGen, &mut src);
            write_program(&dir, &c.prog, c.wrap);
        }
        Some("search") => search(&args),
        Some("check") => {
            // evaluate an existing case directory (h.ml a.ml main.ml) with all configs
            let build = PathBuf::from(arg(&args, "--build").unwrap());
            let dir = PathBuf::from(arg(&args, "--dir").unwrap());
            let dir = fs::canonicalize(&dir).unwrap_or(dir);
            match evaluate(&build, &dir, &configs(), false) {
                Eval::Invalid(m) => println!("invalid case (type error)\n{}", first_lines(&m, 20)),
                Eval::RefTimeout => println!("invalid case (bytecode timeout)"),
                Eval::Done { findings, .. } => {
                    for f in &findings {
                        println!("{} [{}]\n{}\n", f.sig, f.cfg.name, f.message);
                    }
                    if findings.is_empty() {
                        println!("ok");
                    }
                }
            }
        }
        _ => {
            eprintln!("usage: ocfuzz search --build B --seed S --seconds N --out D | gen --seed S --dir D | check --build B --dir D");
            std::process::exit(2);
        }
    }
}

struct Shared {
    st: Stats,
    per_sig: HashMap<String, usize>,
    invalid_kept: usize,
    start: Instant,
    out: PathBuf,
    build: PathBuf,
}

impl Shared {
    /// Writes a finding dir (cap 3 per signature).
    fn record(&mut self, f: &Finding, case: &Case, seed: &str) {
        let prog = &case.prog;
        let n = *self.per_sig.get(&f.sig).unwrap_or(&0);
        if n >= 3 {
            return;
        }
        self.per_sig.insert(f.sig.clone(), n + 1);
        let d = self.out.join("findings").join(&f.sig).join(n.to_string());
        write_program(&d, prog, case.wrap);
        if f.cfg.meta {
            // the re-wrapped A the meta config compiled (repro.sh uses it as a.ml)
            let m = d.join("meta");
            let _ = fs::create_dir_all(&m);
            for x in ["h.ml", "a.ml"] {
                let _ = fs::copy(d.join(x), m.join(x));
            }
            if let Ok(a) = meta_a(&self.build, &m, &prog.a, WRAPS[case.wrap]) {
                let _ = fs::write(d.join("a_meta.ml"), a);
            }
            let _ = fs::write(d.join("main_meta.ml"), meta_main(&prog.main_src(), WRAPS[case.wrap]));
            let _ = fs::remove_dir_all(&m);
        }
        let fj = format!(
            "{{\"signature\": {}, \"oracle\": {}, \"message\": {}, \"seed\": {}, \"config\": {}}}\n",
            json_str(&f.sig),
            json_str(f.sig.split("__").next().unwrap_or("")),
            json_str(&f.message),
            json_str(seed),
            json_str(f.cfg.name)
        );
        let _ = fs::write(d.join("finding.json"), fj);
        let rp = d.join("repro.sh");
        let _ = fs::write(&rp, repro_script(f));
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&rp, fs::Permissions::from_mode(0o755));
    }
}

fn search(args: &[String]) {
    let build = PathBuf::from(arg(args, "--build").unwrap());
    let build = fs::canonicalize(&build).unwrap_or(build);
    let seed: u64 = arg(args, "--seed").unwrap().parse().unwrap();
    let secs: u64 = arg(args, "--seconds").unwrap().parse().unwrap();
    let out = PathBuf::from(arg(args, "--out").unwrap());
    // worker threads, each running its own sequence of ephyra checks (each case
    // compiles its configs in parallel)
    let jobs: usize = arg(args, "--jobs").or_else(|| std::env::var("OCFUZZ_JOBS").ok()).and_then(|s| s.parse().ok()).unwrap_or(3);
    let _ = fs::create_dir_all(out.join("findings"));
    let tmp_root = std::env::temp_dir().join(format!("ocfuzz-{}-{seed}", std::process::id()));
    let cfgs = configs();
    let start = Instant::now();
    let deadline = start + Duration::from_secs(secs.saturating_sub(10).max(1));
    let shared = Mutex::new(Shared { st: Stats::default(), per_sig: HashMap::new(), invalid_kept: 0, start, out: out.clone(), build: build.clone() });
    {
        let sh = shared.lock().unwrap();
        write_stats(&out, &sh.st, start);
    }
    std::thread::scope(|sc| {
        for w in 0..jobs {
            let (build, cfgs, shared, tmp_root) = (&build, &cfgs, &shared, &tmp_root);
            sc.spawn(move || {
                let wroot = tmp_root.join(format!("w{w}"));
                let mut round: u64 = 0;
                while Instant::now() + Duration::from_secs(5) < deadline {
                    let cseed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((w as u64) << 48) ^ round;
                    one_check(build, cfgs, shared, &wroot, cseed, deadline);
                    round += 1;
                    merge_coverage(build);
                }
            });
        }
    });
    {
        let sh = shared.lock().unwrap();
        write_stats(&out, &sh.st, start);
    }
    if std::env::var("BISECT_FILE").is_ok() {
        std::thread::sleep(Duration::from_millis(2100)); // let the last files settle
    }
    merge_coverage(&build);
    let _ = fs::remove_dir_all(&tmp_root);
}

/// State of one `ephyra::check` call: the failure being shrunk and the
/// smallest program that still shows it.
struct Target {
    finding: Finding,
    best: Case,
}

fn one_check(build: &Path, cfgs: &[Cfg], shared: &Mutex<Shared>, wroot: &Path, cseed: u64, deadline: Instant) {
    let dir = wroot.join("case");
    let remaining = deadline.saturating_duration_since(Instant::now());
    let mut config = ephyra::Config::default().tests(TESTS_PER_CHECK).seed(cseed).budget(remaining.min(Duration::from_secs(120))).no_regressions();
    config.shrink.max_steps = 400;
    config.shrink.timeout_ms = Some(remaining.saturating_sub(Duration::from_secs(2)).min(Duration::from_secs(120)).as_millis() as u64);
    let mut target: Option<Target> = None;
    let mut extra: Vec<(Finding, Case)> = vec![];
    let report = ephyra::check(&config, CaseGen, |case: Case| {
        let _ = fs::remove_dir_all(&dir);
        write_program(&dir, &case.prog, case.wrap);
        if let Some(t) = &mut target {
            // rerun / shrink candidate: only the failing config, same check kind
            shared.lock().unwrap().st.shrink_evals += 1;
            if Instant::now() > deadline {
                return; // out of time: stop shrinking (candidate counts as passing)
            }
            let sub = if t.finding.cfg.name == cfgs[0].name { vec![cfgs[0].clone()] } else { vec![cfgs[0].clone(), t.finding.cfg.clone()] };
            if let Eval::Done { findings, .. } = evaluate(build, &dir, &sub, true) {
                let same = findings.iter().find(|f| f.check == t.finding.check && f.cfg.name == t.finding.cfg.name && (f.check == Check::Diff || f.check == Check::RefDiff || f.check == Check::MetaDiff || f.sig == t.finding.sig));
                if let Some(f) = same {
                    let mut f = f.clone();
                    f.sig = t.finding.sig.clone();
                    t.best = case.clone();
                    t.finding.message = f.message.clone();
                    fail(&f);
                }
            }
            return;
        }
        let cc = case_cfgs(cfgs, &case);
        let ev = evaluate(build, &dir, &cc, false);
        let mut sh = shared.lock().unwrap();
        sh.st.cases += 1;
        let start = sh.start;
        match ev {
            Eval::Invalid(m) => {
                sh.st.gen_invalid += 1;
                if sh.invalid_kept < 3 {
                    sh.invalid_kept += 1;
                    let d = sh.out.join(format!("invalid/{cseed}-{}", sh.st.cases));
                    write_program(&d, &case.prog, case.wrap);
                    let _ = fs::write(d.join("INVALID"), m);
                }
                write_stats(&sh.out.clone(), &sh.st, start);
            }
            Eval::RefTimeout => {
                sh.st.ref_timeout += 1;
                write_stats(&sh.out.clone(), &sh.st, start);
            }
            Eval::Done { findings, exn, ref54_skip, meta_skip } => {
                if meta_skip {
                    sh.st.meta_skip += 1;
                }
                if exn {
                    sh.st.ref_exn_cases += 1;
                }
                if ref54_skip {
                    sh.st.ref54_skip += 1;
                }
                if !findings.is_empty() {
                    sh.st.failures += 1;
                }
                write_stats(&sh.out.clone(), &sh.st, start);
                drop(sh);
                if let Some(first) = findings.first() {
                    for f in &findings[1..] {
                        extra.push((f.clone(), case.clone()));
                    }
                    target = Some(Target { finding: first.clone(), best: case.clone() });
                    fail(first);
                }
            }
        }
    });
    let mut sh = shared.lock().unwrap();
    sh.st.checks += 1;
    sh.st.ephyra_tests += report.tests;
    let seed = format!("ephyra:{cseed}");
    if let Some(t) = target {
        sh.record(&t.finding, &t.best, &seed);
    }
    // further failures of the same case, unshrunk
    for (f, c) in extra {
        sh.record(&f, &c, &seed);
    }
    let start = sh.start;
    write_stats(&sh.out.clone(), &sh.st, start);
}

/// With BISECT_FILE set (coverage build), every compiler process writes one
/// `<BISECT_FILE><digits>.coverage`; merge the settled ones into one file and
/// delete the inputs to keep the file count down.
fn merge_coverage(build: &Path) {
    static LOCK: Mutex<u32> = Mutex::new(0);
    let Ok(prefix) = std::env::var("BISECT_FILE") else { return };
    let report = build.join("tools/bin/bisect-ppx-report");
    if !report.exists() {
        return;
    }
    let mut k = LOCK.lock().unwrap();
    *k += 1;
    let pre = PathBuf::from(&prefix);
    let dir = pre.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let dir = fs::canonicalize(&dir).unwrap_or(dir);
    let base = pre.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let now = std::time::SystemTime::now();
    let mut inputs = vec![];
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&base) || !name.ends_with(".coverage") {
                continue;
            }
            let settled = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok()).map(|d| d.as_secs() >= 2).unwrap_or(false);
            if settled {
                inputs.push(e.path());
            }
        }
    }
    if inputs.len() < 2 {
        return;
    }
    let outp = dir.join(format!("{base}merged-{}-{}.coverage", std::process::id(), *k));
    let tmp = dir.join(format!(".ocfuzz-merge-{}-{}.tmp", std::process::id(), *k));
    // run from /: bisect-ppx-report rewrites source paths under its cwd to
    // relative ones, which hides the absolute tree paths the controller matches
    let st = Command::new(&report).current_dir("/").arg("merge").arg(&tmp).args(&inputs).stdout(Stdio::null()).stderr(Stdio::null()).status();
    if matches!(st, Ok(s) if s.success()) && fs::rename(&tmp, &outp).is_ok() {
        for f in inputs {
            let _ = fs::remove_file(f);
        }
    } else {
        let _ = fs::remove_file(&tmp);
    }
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}
