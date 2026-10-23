// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! ocfuzz: differential fuzzer for the OCaml native compiler.
//!
//! Oracles (README.md):
//! - diff: bytecode (ocamlc + ocamlrund) vs ocamlopt in several closure and
//!   flambda configurations must print the same output and exit the same way
//!   (manual/src/cmds/native.etex: "the two compilers should generate code that
//!   behave identically").
//! - crash: a compiler that accepts the program in bytecode must not fail with
//!   a fatal error / assertion / uncaught exception in native, and no binary
//!   may die from a signal or a runtime assertion.
//! - ref: the trunk bytecode program must print the same as the same program
//!   compiled by the OCaml 5.4.0 release (bytecode, an independent front end
//!   and Lambda), which catches front-end bugs bytecode and native share.
mod gen;

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const H_ML: &str = include_str!("h.ml");
const COMPAT54_ML: &str = include_str!("compat54.ml");

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Byte,
    Native,
    /// OCaml 5.4.0 release bytecode (`<build>/ref54/inst`), the ref oracle's reference
    Ref,
}

#[derive(Clone)]
struct Cfg {
    name: &'static str,
    build: &'static str, // closure | flambda
    kind: Kind,
    flags: &'static [&'static str],
}

fn configs() -> Vec<Cfg> {
    vec![
        Cfg { name: "byte", build: "closure", kind: Kind::Byte, flags: &[] },
        Cfg { name: "closure", build: "closure", kind: Kind::Native, flags: &[] },
        Cfg { name: "closure_inline", build: "closure", kind: Kind::Native, flags: &["-inline", "200", "-dcmm-invariants", "-g"] },
        // register allocator / code-size / scheduling toggles must not change results.
        // closure_linscan replaces closure_compact on 1 case in LINSCAN_EVERY (one_case):
        // -linscan miscompiles functions with several atomic ops (trunk, not 5.4.0) on
        // ~20% of cases, which would hide everything else behind it.
        Cfg { name: "closure_compact", build: "closure", kind: Kind::Native, flags: &["-compact", "-no-float-const-prop", "-no-insn-sched", "-function-sections"] },
        Cfg { name: "closure_linscan", build: "closure", kind: Kind::Native, flags: &["-linscan", "-compact", "-no-float-const-prop", "-no-insn-sched", "-function-sections"] },
        Cfg { name: "flambda_o3", build: "flambda", kind: Kind::Native, flags: &["-O3"] },
        Cfg { name: "flambda_classic", build: "flambda", kind: Kind::Native, flags: &["-Oclassic", "-g"] },
        Cfg {
            name: "flambda_o3x",
            build: "flambda",
            kind: Kind::Native,
            flags: &["-O3", "-unbox-closures", "-rounds", "4", "-inline-max-depth", "4", "-remove-unused-arguments", "-runtime-variant", "d"],
        },
        Cfg { name: "ref54", build: "ref54", kind: Kind::Ref, flags: &[] },
    ]
}

const LINSCAN_EVERY: u64 = 32;

// wall clock under the gate load (4 searches x 3 workers x 7 compiles on 16 cpus): heavy but
// terminating flambda -rounds 4 compiles took 27 s alone and > 60 s loaded
const COMPILE_TIMEOUT: u64 = 150;
const RUN_TIMEOUT: u64 = 10;

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
    let _ = fs::create_dir_all(&d);
    for f in ["h.ml", "a.ml", "main.ml"] {
        let _ = fs::copy(dir.join(f), d.join(f));
    }
    let bin = build.join(c.build).join("inst/bin");
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
    cmd.current_dir(&d);
    Res::Ran(run_cmd(&mut cmd, RUN_TIMEOUT))
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
        _ => "x",
    }
}

fn group(build: &str) -> &str {
    build
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
    /// cases where the ref oracle was skipped: min_int constant pattern (5.4.0 bug)
    ref_skip_min_int: u64,
    reduced: u64,
}

struct Finding {
    sig: String,
    message: String,
    cfg: Cfg,
    /// crash at compile time (repro checks compiler status), else run diff/crash
    check: Check,
}

#[derive(Clone, Copy, PartialEq)]
enum Check {
    CompileCrash,
    CompileTimeout,
    RunSignal,
    RunTimeout,
    Diff,
    RefCrash,
    /// trunk bytecode differs from the 5.4.0 release bytecode
    RefDiff,
}

/// Runs all configs over the program in `dir`; returns findings (one per
/// failing config, at most) or Err(()) if the case is invalid (skip).
fn evaluate(build: &Path, dir: &Path, cfgs: &[Cfg], st: &mut Stats) -> Result<Vec<Finding>, ()> {
    let results: Vec<Res> = std::thread::scope(|sc| {
        let hs: Vec<_> = cfgs.iter().map(|c| sc.spawn(move || compile_and_run(build, dir, c))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut fs_ = vec![];
    let reference = &results[0];
    let ref_out = match reference {
        Res::CompileFail(m) => {
            if is_compiler_crash(m) {
                fs_.push(Finding {
                    sig: format!("crash__compile__byte_{}", crash_area(m)),
                    message: first_lines(m, 20),
                    cfg: cfgs[0].clone(),
                    check: Check::CompileCrash,
                });
                return Ok(fs_);
            }
            st.gen_invalid += 1;
            let _ = fs::write(dir.join("INVALID"), m);
            return Err(());
        }
        Res::CompileTimeout => {
            fs_.push(Finding { sig: "hang__compile__byte".into(), message: "ocamlc timeout".into(), cfg: cfgs[0].clone(), check: Check::CompileTimeout });
            return Ok(fs_);
        }
        Res::Ran(o) => {
            if o.status.is_none() {
                st.ref_timeout += 1;
                let _ = fs::write(dir.join("INVALID"), "reference (bytecode) run timed out\n");
                return Err(());
            }
            if o.signal.is_some() || o.stderr.contains("Assertion") || o.stderr.contains("Fatal error") {
                fs_.push(Finding {
                    sig: format!("crash__run__byte_{}", run_crash_area(o)),
                    message: first_lines(&o.stderr, 20),
                    cfg: cfgs[0].clone(),
                    check: Check::RefCrash,
                });
                return Ok(fs_);
            }
            if o.stdout.contains("!exn") {
                st.ref_exn_cases += 1;
            }
            outcome(o)
        }
    };
    let mut outs: Vec<(&Cfg, String)> = vec![(&cfgs[0], ref_out.clone())];
    for (c, r) in cfgs.iter().zip(&results).skip(1) {
        if c.kind == Kind::Ref {
            continue;
        }
        let g = group(c.build);
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
    // ref: trunk bytecode vs the 5.4.0 release. The area says which trunk
    // configs differ from 5.4.0: `all` (bytecode and every native config, i.e.
    // the shared front end / Lambda), `byte` (bytecode only; also a diff
    // finding), else the tags. A 5.4.0 failure to compile, crash or timeout
    // skips the oracle (not the target).
    // Known 5.4.0 bug, fixed in trunk (Changes: "#14890, fix pattern
    // compilation in presence of the [min_int] constant pattern", commit
    // 60c029277 "switch: fix overflow on min_int"): an integer switch whose
    // lowest constant is min_int sends other values to the min_int arm in
    // 5.4.0. The ref oracle only compares the tests that run before the first
    // test that can reach such a pattern (counted in ref_skip_min_int).
    let min_int_cut = min_int_cut(dir);
    if min_int_cut.is_some() && cfgs.iter().any(|c| c.kind == Kind::Ref) {
        st.ref_skip_min_int += 1;
    }
    let cut = |s: &str| -> String {
        match &min_int_cut {
            Some(t) => s.split(&format!("\n=={}\n", t)).next().unwrap_or("").to_string(),
            None => s.to_string(),
        }
    };
    let ref_out = cut(&ref_out);
    let outs: Vec<(&Cfg, String)> = outs.into_iter().map(|(c, x)| (c, cut(&x))).collect();
    if let Some((rc, Res::Ran(o))) = cfgs.iter().zip(&results).find(|(c, _)| c.kind == Kind::Ref) {
        if o.status.is_some() && o.signal.is_none() {
            let r54 = cut(&outcome(o));
            if r54 != ref_out {
                let differ: Vec<&str> = outs.iter().filter(|(_, x)| *x != r54).map(|(c, _)| if c.kind == Kind::Byte { "b" } else { short(c.name) }).collect();
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
    // one finding per signature per case
    let mut seen = std::collections::HashSet::new();
    fs_.retain(|f| seen.insert(f.sig.clone()));
    Ok(fs_)
}

/// The literal min_int constant pattern (gen.rs int_pat is its only source).
const MIN_INT_PAT: &str = "(-4611686018427387904)";

/// Name ("tN") of the first test in run order that can reach a min_int
/// constant pattern: its own text contains one, or it references an A
/// definition that does (transitively through earlier A definitions).
fn min_int_cut(dir: &Path) -> Option<String> {
    let a = fs::read_to_string(dir.join("a.ml")).unwrap_or_default();
    let main = fs::read_to_string(dir.join("main.ml")).unwrap_or_default();
    if !a.contains(MIN_INT_PAT) && !main.contains(MIN_INT_PAT) {
        return None;
    }
    let refs = |text: &str, name: &str, qual: &str| -> bool {
        let pat = format!("{qual}{name}");
        text.match_indices(&pat).any(|(i, _)| {
            let after = text[i + pat.len()..].chars().next();
            let before = text[..i].chars().last();
            !after.map_or(false, |c| c.is_ascii_alphanumeric() || c == '_')
                && (qual != "" || !before.map_or(false, |c| c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        })
    };
    // a.ml top-level definitions are g<N> (globals) and fa<N> (functions);
    // other column-0 lets are locals of the preceding top-level definition.
    let mut defs: Vec<(String, String)> = vec![];
    for line in a.lines() {
        let top = line.strip_prefix("let").and_then(|r| {
            let r = if r.starts_with("[@") { &r[r.find(']')? + 1..] } else { r };
            let r = r.strip_prefix(' ')?;
            let name: String = r.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            let digits = name.trim_start_matches("fa").trim_start_matches('g');
            ((name.starts_with("fa") || name.starts_with('g')) && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())).then_some(name)
        });
        match top {
            Some(n) => defs.push((n, format!("{line}\n"))),
            None => match defs.last_mut() {
                Some(d) => {
                    d.1.push_str(line);
                    d.1.push('\n');
                }
                None => return Some("t0".into()),
            },
        }
    }
    let mut tainted: Vec<&str> = vec![];
    for (n, body) in &defs {
        if body.contains(MIN_INT_PAT) || tainted.iter().any(|t| refs(body, t, "")) {
            tainted.push(n);
        }
    }
    // main.ml: "let tN () =" sections, run in order of the H.run lines
    let mut sec: Vec<(String, String)> = vec![];
    for line in main.lines() {
        if let Some(r) = line.strip_prefix("let t") {
            if let Some(n) = r.strip_suffix(" () =") {
                sec.push((format!("t{n}"), String::new()));
                continue;
            }
        }
        if let Some(s) = sec.last_mut() {
            s.1.push_str(line);
            s.1.push('\n');
        }
    }
    let bad: Vec<&str> = sec
        .iter()
        .filter(|(_, b)| b.contains(MIN_INT_PAT) || tainted.iter().any(|t| refs(b, t, "A.")))
        .map(|(n, _)| n.as_str())
        .collect();
    main.lines()
        .filter_map(|l| l.strip_prefix("let () = H.run \"").and_then(|r| r.split('"').next()))
        .find(|n| bad.contains(n))
        .map(|n| n.to_string())
}

fn first_lines(s: &str, n: usize) -> String {
    let v: Vec<&str> = s.lines().take(n).collect();
    let mut r = v.join("\n");
    if r.len() > 2000 {
        r.truncate(2000);
    }
    r
}

fn write_program(dir: &Path, p: &gen::Program, keep: Option<&[usize]>) {
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(dir.join("h.ml"), H_ML);
    let _ = fs::write(dir.join("a.ml"), &p.a);
    let _ = fs::write(dir.join("main.ml"), p.main_src(keep));
    let _ = fs::write(dir.join("compat54.ml"), COMPAT54_ML);
}

fn repro_script(f: &Finding) -> String {
    let c = &f.cfg;
    let flags = c.flags.join(" ");
    let comp = if c.kind == Kind::Byte { "ocamlc" } else { "ocamlopt" };
    let exe = if c.kind == Kind::Byte { "p.byte" } else { "p.exe" };
    let runexe = if c.kind == Kind::Byte { format!("\"$B/{}/inst/bin/ocamlrund\" ./p.byte", c.build) } else { "./p.exe".into() };
    let head = format!(
        r#"#!/bin/bash
# Reproduces: {sig}
# usage: repro.sh <build-dir>   (exit 1: reproduces, 0: does not, 2: cannot decide)
B=$(realpath "$1")
ulimit -c 0
cd "$(dirname "$0")"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
cp h.ml a.ml main.ml compat54.ml "$T"/
cd "$T"
CB="$B/closure/inst/bin"
TB="$B/{build}/inst/bin"
"#,
        sig = f.sig,
        build = c.build
    );
    let body = match f.check {
        Check::CompileCrash => format!(
            r#""$CB/ocamlc" -w -a h.ml a.ml main.ml -o ref.byte > ref.log 2>&1 || {{ [ "{comp}" = ocamlc ] || exit 2; }}
timeout {COMPILE_TIMEOUT} "$TB/{comp}" -w -a {flags} h.ml a.ml main.ml -o {exe} > c.log 2>&1
st=$?
[ $st -eq 124 ] && exit 2
[ $st -eq 0 ] && exit 0
grep -qE 'Fatal error|Assert_failure|Uncaught exception|Stack_overflow|Out of memory|Segmentation|Error during linking|Assembler error' c.log && exit 1
exit 2
"#
        ),
        Check::CompileTimeout => format!(
            r#""$CB/ocamlc" -w -a h.ml a.ml main.ml -o ref.byte > ref.log 2>&1 || {{ [ "{comp}" = ocamlc ] || exit 2; }}
timeout {COMPILE_TIMEOUT} "$TB/{comp}" -w -a {flags} h.ml a.ml main.ml -o {exe} > c.log 2>&1
[ $? -eq 124 ] && exit 1
exit 0
"#
        ),
        Check::RefCrash => format!(
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
timeout {RUN_TIMEOUT} {runexe} > out.txt 2> err.txt
st=$?
{decide}
"#
            )
        }
    };
    head + &body
}

fn case_seed(seed: u64, i: u64) -> u64 {
    let mut r = gen::Rng::new(seed.wrapping_mul(0x1000_0000_01B3) ^ i.wrapping_mul(0x9E37_79B9));
    r.next()
}

fn write_stats(out: &Path, st: &Stats, start: Instant) {
    let el = start.elapsed().as_secs_f64();
    let s = format!(
        "{{\"cases\": {}, \"failures\": {}, \"gen_invalid\": {}, \"ref_timeout\": {}, \"ref_exn_cases\": {}, \"ref_skip_min_int\": {}, \"reduced\": {}, \"elapsed_s\": {:.1}, \"cases_per_s\": {:.3}}}\n",
        st.cases,
        st.failures,
        st.gen_invalid,
        st.ref_timeout,
        st.ref_exn_cases,
        st.ref_skip_min_int,
        st.reduced,
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("gen") => {
            let seed: u64 = arg(&args, "--seed").unwrap().parse().unwrap();
            let dir = PathBuf::from(arg(&args, "--dir").unwrap());
            let p = gen::Gen::new(seed).program();
            write_program(&dir, &p, None);
        }
        Some("search") => search(&args),
        Some("check") => {
            // evaluate an existing case directory (h.ml a.ml main.ml) with all configs
            let build = PathBuf::from(arg(&args, "--build").unwrap());
            let dir = PathBuf::from(arg(&args, "--dir").unwrap());
            // absolute: native binaries are started by path from inside their config dir
            let dir = fs::canonicalize(&dir).unwrap_or(dir);
            let mut st = Stats::default();
            match evaluate(&build, &dir, &configs(), &mut st) {
                Err(()) => println!("invalid case (type error or reference timeout)"),
                Ok(fs_) => {
                    for f in &fs_ {
                        println!("{} [{}]\n{}\n", f.sig, f.cfg.name, f.message);
                        if let Some(o) = arg(&args, "--emit") {
                            // write a finding dir (as search would) for testing repro.sh
                            let d = PathBuf::from(o).join(&f.sig);
                            let _ = fs::create_dir_all(&d);
                            for x in ["h.ml", "a.ml", "main.ml", "compat54.ml"] {
                                let _ = fs::copy(dir.join(x), d.join(x));
                            }
                            let _ = fs::write(d.join("repro.sh"), repro_script(f));
                        }
                    }
                    if fs_.is_empty() {
                        println!("ok");
                    }
                }
            }
        }
        _ => {
            eprintln!("usage: ocfuzz search --build B --seed S --seconds N --out D | gen --seed S --dir D");
            std::process::exit(2);
        }
    }
}

struct Shared {
    st: Stats,
    per_sig: HashMap<String, usize>,
    invalid_kept: usize,
}

fn search(args: &[String]) {
    let build = PathBuf::from(arg(args, "--build").unwrap());
    let seed: u64 = arg(args, "--seed").unwrap().parse().unwrap();
    let secs: u64 = arg(args, "--seconds").unwrap().parse().unwrap();
    let out = PathBuf::from(arg(args, "--out").unwrap());
    // worker threads, each running whole cases (each case compiles its configs in parallel)
    let jobs: usize = arg(args, "--jobs").or_else(|| std::env::var("OCFUZZ_JOBS").ok()).and_then(|s| s.parse().ok()).unwrap_or(3);
    let _ = fs::create_dir_all(out.join("findings"));
    let tmp_root = std::env::temp_dir().join(format!("ocfuzz-{}-{seed}", std::process::id()));
    let cfgs = configs();
    let start = Instant::now();
    let deadline = start + Duration::from_secs(secs.saturating_sub(8).max(1));
    let shared = std::sync::Mutex::new(Shared { st: Stats::default(), per_sig: HashMap::new(), invalid_kept: 0 });
    let next = std::sync::atomic::AtomicU64::new(0);
    write_stats(&out, &shared.lock().unwrap().st, start);
    std::thread::scope(|sc| {
        for w in 0..jobs {
            let (build, out, cfgs, shared, next, tmp_root) = (&build, &out, &cfgs, &shared, &next, &tmp_root);
            sc.spawn(move || {
                let wroot = tmp_root.join(format!("w{w}"));
                while Instant::now() < deadline {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    one_case(build, out, cfgs, shared, &wroot, case_seed(seed, i), start);
                    if i % 20 == 19 {
                        merge_coverage(build);
                    }
                }
            });
        }
    });
    write_stats(&out, &shared.lock().unwrap().st, start);
    if std::env::var("BISECT_FILE").is_ok() {
        std::thread::sleep(Duration::from_millis(2100)); // let the last files settle
    }
    merge_coverage(&build);
    let _ = fs::remove_dir_all(&tmp_root);
}

/// With BISECT_FILE set (coverage build), every compiler process writes one
/// `<BISECT_FILE><digits>.coverage`; merge the settled ones into one file and
/// delete the inputs to keep the file count down.
fn merge_coverage(build: &Path) {
    static LOCK: std::sync::Mutex<u32> = std::sync::Mutex::new(0);
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

fn one_case(build: &Path, out: &Path, cfgs: &[Cfg], shared: &std::sync::Mutex<Shared>, wroot: &Path, cs: u64, start: Instant) {
    let dir = wroot.join("case");
    let _ = fs::remove_dir_all(&dir);
    let prog = gen::Gen::new(cs).program();
    write_program(&dir, &prog, None);
    let mut lst = Stats::default();
    let drop = if cs % LINSCAN_EVERY == 0 { "closure_compact" } else { "closure_linscan" };
    let case_cfgs: Vec<Cfg> = cfgs.iter().filter(|c| c.name != drop).cloned().collect();
    let cfgs = &case_cfgs[..];
    let res = evaluate(build, &dir, cfgs, &mut lst);
    let findings = match res {
        Err(()) => {
            let mut sh = shared.lock().unwrap();
            sh.st.cases += 1;
            sh.st.gen_invalid += lst.gen_invalid;
            sh.st.ref_timeout += lst.ref_timeout;
            if dir.join("INVALID").exists() && sh.invalid_kept < 3 {
                sh.invalid_kept += 1;
                let d = out.join(format!("invalid/{cs}"));
                let _ = fs::create_dir_all(&d);
                for f in ["h.ml", "a.ml", "main.ml", "INVALID"] {
                    let _ = fs::copy(dir.join(f), d.join(f));
                }
            }
            write_stats(out, &sh.st, start);
            return;
        }
        Ok(f) => f,
    };
    // reduce outside the lock: keep only the first differing test if that still reproduces
    let mut reduced: Vec<Option<Vec<usize>>> = vec![];
    for f in &findings {
        let mut keep = None;
        let full = shared.lock().unwrap().per_sig.get(&f.sig).copied().unwrap_or(0) >= 3;
        if !full && (f.check == Check::Diff || f.check == Check::RunSignal || f.check == Check::RefDiff) {
            if let Some(t) = f.message.split("test Some(").nth(1).and_then(|s| s.split(')').next()).and_then(|s| s.parse::<usize>().ok()) {
                let rdir = wroot.join("reduce");
                let _ = fs::remove_dir_all(&rdir);
                write_program(&rdir, &prog, Some(&[t]));
                let sub = vec![cfgs[0].clone(), f.cfg.clone()];
                let mut dummy = Stats::default();
                if let Ok(fs2) = evaluate(build, &rdir, &sub, &mut dummy) {
                    if fs2.iter().any(|g| g.sig == f.sig || (f.check == Check::Diff && g.check == Check::Diff) || (f.check == Check::RefDiff && g.check == Check::RefDiff)) {
                        keep = Some(vec![t]);
                    }
                }
            }
        }
        reduced.push(keep);
    }
    let mut sh = shared.lock().unwrap();
    sh.st.cases += 1;
    sh.st.ref_exn_cases += lst.ref_exn_cases;
    sh.st.ref_skip_min_int += lst.ref_skip_min_int;
    if !findings.is_empty() {
        sh.st.failures += 1;
    }
    for (f, keep) in findings.iter().zip(reduced) {
        let n = *sh.per_sig.get(&f.sig).unwrap_or(&0);
        if n >= 3 {
            continue;
        }
        sh.per_sig.insert(f.sig.clone(), n + 1);
        if keep.is_some() {
            sh.st.reduced += 1;
        }
        let d = out.join("findings").join(&f.sig).join(n.to_string());
        write_program(&d, &prog, keep.as_deref());
        let fj = format!(
            "{{\"signature\": {}, \"oracle\": {}, \"message\": {}, \"seed\": {}, \"config\": {}}}\n",
            json_str(&f.sig),
            json_str(f.sig.split("__").next().unwrap_or("")),
            json_str(&f.message),
            cs,
            json_str(f.cfg.name)
        );
        let _ = fs::write(d.join("finding.json"), fj);
        let rp = d.join("repro.sh");
        let _ = fs::write(&rp, repro_script(f));
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&rp, fs::Permissions::from_mode(0o755));
    }
    write_stats(out, &sh.st, start);
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
