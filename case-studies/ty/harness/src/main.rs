// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! tyz: generated typed Python programs against ty (README.md).
//!
//!   tyz search <seed> <secs> <out>   batches of programs; findings under <out>/findings
//!   tyz gen <seed> <n> <dir>         write n programs (p_<i>.py) for inspection
//!   tyz eval <file.py>               one program: prints the verdict, exit 1 when it fails
//!
//! Environment: TYZ_TY (ty binary), TYZ_PY (python 3.13+), TYZ_WORKERS, TYZ_BATCH.

mod pygen;
mod tree;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ephyra::{Config, Gen, GenExt, Source, TestResult, check};
use serde_json::{Value, json};

use tree::Tree;

const RT: &str = include_str!("../py/_rt.py");
const RUNNER: &str = include_str!("../py/runner.py");

fn ty_bin() -> String {
    std::env::var("TYZ_TY").unwrap_or_else(|_| "ty".into())
}
fn py_bin() -> String {
    std::env::var("TYZ_PY").unwrap_or_else(|_| "python3".into())
}

#[derive(Clone, Debug)]
enum Verdict {
    /// The program does not run cleanly under CPython (a generator miss).
    Invalid(String),
    Pass { probes: usize, asserts: usize },
    /// `errs`: the error diagnostics on program lines, as `rule: message`.
    Fail { sig: String, msg: String, errs: Vec<String> },
}

/// The check file: the program with `static_assert`s after each observed probe.
/// Returns the source and the 1-based line numbers of the inserted lines.
fn check_file(src: &str, obs: &BTreeMap<String, Vec<String>>) -> (String, BTreeSet<usize>, usize) {
    let mut out = String::new();
    let mut inserted = BTreeSet::new();
    let mut n = 0usize;
    let mut asserts = 0;
    let mut header_done = false;
    let header = "import typing as _t\nimport ty_extensions as _tx\nfrom ty_extensions import static_assert\nfrom ty_extensions._internal import TypeOf, is_assignable_to, is_disjoint_from\n";
    let mut push = |out: &mut String, line: &str, ins: bool, n: &mut usize| {
        out.push_str(line);
        out.push('\n');
        *n += 1;
        if ins {
            inserted.insert(*n);
        }
    };
    for line in src.lines() {
        if !header_done && !line.starts_with("from __future__") {
            for h in header.lines() {
                push(&mut out, h, true, &mut n);
            }
            header_done = true;
        }
        push(&mut out, line, false, &mut n);
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("_probe(") {
            let ind = &line[..line.len() - t.len()];
            let (k, e) = rest.split_once(", ").unwrap();
            let e = e.strip_suffix(')').unwrap();
            if let Some(tys) = obs.get(k) {
                if !tys.is_empty() && !tys.iter().any(|t| t == "?") {
                    let u = if tys.len() == 1 { tys[0].clone() } else { format!("_t.Union[{}]", tys.join(", ")) };
                    // Disjointness fails only if ty rules out the runtime value itself
                    // (`u` contains it however coarse it is); assignability also fails
                    // when ty cannot prove a coarse `u` fits, so it needs triage.
                    push(&mut out, &format!("{ind}static_assert(not is_disjoint_from({u}, TypeOf[{e}]))  # tyz:{k}:d"), true, &mut n);
                    push(&mut out, &format!("{ind}static_assert(is_assignable_to({u}, TypeOf[{e}]))  # tyz:{k}:a"), true, &mut n);
                    asserts += 1;
                }
            }
        }
    }
    (out, inserted, asserts)
}

struct Diag {
    line: usize,
    severity: String,
    rule: String,
    msg: String,
}

fn parse_diags(out: &str) -> BTreeMap<String, Vec<Diag>> {
    let mut by_file: BTreeMap<String, Vec<Diag>> = BTreeMap::new();
    for l in out.lines() {
        // path:line:col: severity[rule] message
        let mut it = l.splitn(4, ':');
        let (Some(path), Some(line), Some(_col), Some(rest)) = (it.next(), it.next(), it.next(), it.next()) else { continue };
        let Ok(line) = line.parse::<usize>() else { continue };
        let rest = rest.trim_start();
        let Some((sev, rest)) = rest.split_once('[') else { continue };
        let Some((rule, msg)) = rest.split_once("] ") else { continue };
        let file = Path::new(path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        by_file.entry(file).or_default().push(Diag { line, severity: sev.to_string(), rule: rule.to_string(), msg: msg.to_string() });
    }
    by_file
}

fn panic_sig(msg: &str) -> String {
    // "Panicked at crates/x/src/y.rs:12:5 when checking ..."
    let loc = msg.strip_prefix("Panicked at ").and_then(|r| r.split_whitespace().next()).unwrap_or("unknown");
    let mut parts: Vec<&str> = loc.rsplitn(3, ':').collect();
    parts.reverse();
    let file = parts.first().copied().unwrap_or("unknown");
    let line = parts.get(1).copied().unwrap_or("0");
    let short = file.rsplit('/').next().unwrap_or(file).trim_end_matches(".rs");
    format!("crash__panic__{short}_{line}")
}

/// Runs a batch of programs: CPython, then ty on the check files.
fn eval_batch(dir: &Path, srcs: &[String]) -> Vec<Verdict> {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("_rt.py"), RT).unwrap();
    fs::write(dir.join("runner.py"), RUNNER).unwrap();
    let names: Vec<String> = (0..srcs.len()).map(|i| format!("p_{i}.py")).collect();
    for (n, s) in names.iter().zip(srcs) {
        fs::write(dir.join(n), s).unwrap();
    }
    let out = Command::new(py_bin()).arg("-I").arg(dir.join("runner.py")).arg(dir).args(&names).stderr(Stdio::null()).output().expect("python");
    let mut runs: BTreeMap<String, Value> = BTreeMap::new();
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        if let Ok(v) = serde_json::from_str::<Value>(l) {
            let f = Path::new(v["file"].as_str().unwrap_or("")).file_name().unwrap().to_string_lossy().to_string();
            runs.insert(f, v);
        }
    }
    // Check files for the programs that ran.
    let cdir = dir.join("check");
    fs::create_dir_all(&cdir).unwrap();
    fs::write(cdir.join("_rt.py"), "def _probe(k: int, v: object) -> None: ...\n").unwrap();
    let mut verdicts: Vec<Option<Verdict>> = vec![None; srcs.len()];
    let mut meta: Vec<(BTreeSet<usize>, usize, usize)> = vec![(BTreeSet::new(), 0, 0); srcs.len()];
    let mut any = false;
    for (i, n) in names.iter().enumerate() {
        let Some(r) = runs.get(n) else {
            verdicts[i] = Some(Verdict::Invalid("runner lost".into()));
            continue;
        };
        if r["status"] != "ok" {
            let e = r["error"].as_str().unwrap_or(r["status"].as_str().unwrap_or("?"));
            verdicts[i] = Some(Verdict::Invalid(e.to_string()));
            continue;
        }
        let obs: BTreeMap<String, Vec<String>> = serde_json::from_value(r["obs"].clone()).unwrap_or_default();
        let (c, ins, asserts) = check_file(&srcs[i], &obs);
        fs::write(cdir.join(format!("c_{i}.py")), c).unwrap();
        meta[i] = (ins, asserts, obs.len());
        any = true;
    }
    if !any {
        return verdicts.into_iter().map(|v| v.unwrap()).collect();
    }
    let t0 = Instant::now();
    let mut child = Command::new(ty_bin())
        .args(["check", "--python-version", "3.13", "--output-format", "concise", "--no-progress", "--project", "."])
        .current_dir(&cdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ty");
    let timeout = Duration::from_secs(60 + srcs.len() as u64);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break Some(s);
        }
        if t0.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    use std::io::Read;
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut stdout);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut stderr);
    }
    let diags = parse_diags(&stdout);
    let batch_fail = match status {
        None => Some(("crash__hang__ty".to_string(), format!("ty ran past {timeout:?}"))),
        Some(s) if s.code().is_none_or(|c| c > 1) => Some((format!("crash__exit__ty_{}", s.code().map(|c| c.to_string()).unwrap_or("signal".into())), stderr.chars().take(2000).collect())),
        _ => None,
    };
    for i in 0..srcs.len() {
        if verdicts[i].is_some() {
            continue;
        }
        if let Some((sig, msg)) = &batch_fail {
            // Attributed to every program of the batch; the shrinker isolates it.
            verdicts[i] = Some(Verdict::Fail { sig: sig.clone(), msg: msg.clone(), errs: vec![] });
            continue;
        }
        let (ins, asserts, probes) = &meta[i];
        let empty = Vec::new();
        let ds = diags.get(&format!("c_{i}.py")).unwrap_or(&empty);
        let fatal = ds.iter().find(|d| d.rule == "panic" || d.severity == "fatal");
        let base_err = ds.iter().find(|d| d.severity == "error" && !ins.contains(&d.line));
        let ins_other = ds.iter().find(|d| d.severity == "error" && ins.contains(&d.line) && d.rule != "static-assert-error");
        let src = fs::read_to_string(cdir.join(format!("c_{i}.py"))).unwrap_or_default();
        let lines: Vec<&str> = src.lines().collect();
        let line_of = |d: &Diag| lines.get(d.line - 1).copied().unwrap_or("").trim().to_string();
        let sound_d = ds.iter().find(|d| d.rule == "static-assert-error" && ins.contains(&d.line) && line_of(d).ends_with(":d"));
        let sound = ds.iter().find(|d| d.rule == "static-assert-error" && ins.contains(&d.line));
        let errs: Vec<String> = ds.iter().filter(|d| d.severity == "error" && !ins.contains(&d.line)).map(|d| format!("{}: {}", d.rule, d.msg)).collect();
        let errs2 = errs.clone();
        verdicts[i] = Some(if let Some(d) = fatal {
            Verdict::Fail { sig: panic_sig(&d.msg), msg: d.msg.clone(), errs: errs2 }
        } else if let Some(d) = base_err {
            Verdict::Fail { sig: format!("fp__{}", d.rule), msg: format!("line {}: {}", d.line, d.msg), errs: errs2 }
        } else if let Some(d) = ins_other {
            Verdict::Fail { sig: format!("harness__assert__{}", d.rule), msg: format!("line {}: {}", d.line, d.msg), errs: errs2 }
        } else if let Some(d) = sound_d {
            Verdict::Fail { sig: "sound__disjoint".into(), msg: format!("line {}: {}", d.line, line_of(d)), errs: errs2 }
        } else if let Some(d) = sound {
            Verdict::Fail { sig: "sound__assign".into(), msg: format!("line {}: {}", d.line, line_of(d)), errs: errs2 }
        } else {
            Verdict::Pass { probes: *probes, asserts: *asserts }
        });
    }
    verdicts.into_iter().map(|v| v.unwrap()).collect()
}

fn program_gen() -> impl ephyra::Gen<Value = Tree> {
    |g: &mut Source| pygen::Model::program(g)
}

/// Shrinks `prog` with ephyra, keeping failures of signature `sig`.
/// For false positives a candidate must keep only errors the original had, so the
/// shrinker cannot trade the finding for an unrelated error of the same rule.
fn minimize(prog: &Tree, sig: &str, orig_errs: &[String], dir: &Path, secs: u64) -> (Tree, String) {
    let p0 = prog.clone();
    let allowed: BTreeSet<String> = orig_errs.iter().cloned().collect();
    let generator = (move |_g: &mut Source| p0.clone()).with_shrink(|t: &Tree, visit| t.shrink(visit)).boxed();
    let mut cfg = Config::default().name("tyz-minimize").tests(1).seed(1);
    cfg.regressions.save = false;
    cfg.regressions.replay = false;
    cfg.shrink.max_steps = 5000;
    cfg.shrink.timeout_ms = Some(secs * 1000);
    cfg.coverage.enabled = false;
    cfg.report.samples = 0;
    let best: RefCell<(Tree, String)> = RefCell::new((prog.clone(), String::new()));
    let _ = check(&cfg, generator, |t: Tree| {
        match eval_batch(dir, &[t.source()]).pop().unwrap() {
            Verdict::Fail { sig: s, msg, errs } if s == sig && (!sig.starts_with("fp__") || errs.iter().all(|e| allowed.contains(e))) => {
                let mut b = best.borrow_mut();
                if t.size() <= b.0.size() {
                    *b = (t, msg.clone());
                }
                TestResult::Fail(msg)
            }
            _ => TestResult::Pass,
        }
    });
    best.into_inner()
}

/// Findings already triaged (`known.json`): a signature prefix and a substring of
/// the message or of one of the program's errors.
#[derive(serde::Deserialize)]
struct Known {
    name: String,
    sig: String,
    msg: String,
}

/// The triaged findings compiled in (`known.json`), so a built binary is
/// self-contained; TYZ_KNOWN names another file.
const KNOWN: &str = include_str!("../known.json");

fn load_known() -> Vec<Known> {
    let text = match std::env::var("TYZ_KNOWN") {
        Ok(path) => fs::read_to_string(path).unwrap_or_default(),
        Err(_) => KNOWN.to_string(),
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn known_name<'a>(known: &'a [Known], sig: &str, msg: &str, errs: &[String]) -> Option<&'a str> {
    known.iter().find(|k| sig.starts_with(&k.sig) && (msg.contains(&k.msg) || errs.iter().any(|e| e.contains(&k.msg)))).map(|k| k.name.as_str())
}

/// Each finding's `repro.sh <build-dir>` (the ephyra-agent contract): exit 1 when
/// `original.py` still fails with the finding's signature against that build's ty.
const REPRO_SH: &str = r#"#!/bin/sh
# repro.sh <build-dir>: exit 1 = original.py still fails with this finding's signature,
# 0 = it passes, 2 = cannot decide (the program no longer runs cleanly under CPython).
here=$(cd "$(dirname "$0")" && pwd)
b=$(cd "$1" && pwd)
exec "$b/bin/tyz" repro "$here" "$b"
"#;

#[derive(Default)]
struct Stats {
    cases: u64,
    invalid: u64,
    pass: u64,
    probes: u64,
    asserts: u64,
    fails: BTreeMap<String, u64>,
    invalid_kinds: BTreeMap<String, u64>,
    batches: u64,
    ms_eval: u64,
}

fn search(seed: u64, secs: u64, out: &Path) {
    let workers: u64 = std::env::var("TYZ_WORKERS").ok().and_then(|s| s.parse().ok()).unwrap_or(4);
    let batch: usize = std::env::var("TYZ_BATCH").ok().and_then(|s| s.parse().ok()).unwrap_or(48);
    let deadline = Instant::now() + Duration::from_secs(secs);
    // TYZ_MAX_CASES: stop after this many programs (per worker share), so coverage
    // comparisons see the same programs whatever the machine's load.
    let max_cases: u64 = std::env::var("TYZ_MAX_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(u64::MAX);
    let per_worker_rounds = max_cases.div_ceil(workers * batch as u64);
    fs::create_dir_all(out.join("findings")).unwrap();
    let stats = Arc::new(Mutex::new(Stats::default()));
    let seen: Arc<Mutex<BTreeMap<String, u32>>> = Arc::new(Mutex::new(BTreeMap::new()));
    let mut hs = Vec::new();
    for w in 0..workers {
        let (stats, seen, out) = (stats.clone(), seen.clone(), out.to_path_buf());
        hs.push(std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
            let known = load_known();
            let dir = out.join(format!("work-{w}"));
            let mut round = 0u64;
            while Instant::now() < deadline && round < per_worker_rounds {
                let base = seed.wrapping_mul(1_000_003).wrapping_add(w * 1_000_000_007 + round * batch as u64);
                round += 1;
                let progs: Vec<(u64, Tree)> = (0..batch)
                    .map(|i| {
                        let s = base + i as u64;
                        let mut src = Source::new(s, 30 + (i * 70 / batch));
                        (s, program_gen().generate(&mut src))
                    })
                    .collect();
                let srcs: Vec<String> = progs.iter().map(|(_, t)| t.source()).collect();
                let t0 = Instant::now();
                let vs = eval_batch(&dir.join("batch"), &srcs);
                let ms = t0.elapsed().as_millis() as u64;
                let mut todo: Vec<(String, String, u64, Tree, u32, Vec<String>)> = Vec::new();
                {
                    let mut st = stats.lock().unwrap();
                    st.batches += 1;
                    st.ms_eval += ms;
                    for (v, (s, t)) in vs.iter().zip(&progs) {
                        st.cases += 1;
                        match v {
                            Verdict::Invalid(e) => {
                                st.invalid += 1;
                                // Keep the first few generator misses for inspection.
                                if st.invalid <= 20 {
                                    let d = out.join("invalid");
                                    let _ = fs::create_dir_all(&d);
                                    let _ = fs::write(d.join(format!("{s}.py")), format!("# {}\n{}", e.lines().next().unwrap_or(""), t.source()));
                                }
                                let k = e.split(':').next().unwrap_or(e.as_str()).to_string();
                                *st.invalid_kinds.entry(k).or_default() += 1;
                            }
                            &Verdict::Pass { probes, asserts } => {
                                st.pass += 1;
                                st.probes += probes as u64;
                                st.asserts += asserts as u64;
                            }
                            Verdict::Fail { sig, msg, errs } => {
                                if let Some(k) = known_name(&known, sig, msg, errs) {
                                    *st.fails.entry(format!("known__{k}")).or_default() += 1;
                                    continue;
                                }
                                *st.fails.entry(sig.clone()).or_default() += 1;
                                let mut sn = seen.lock().unwrap();
                                let c = sn.entry(sig.clone()).or_default();
                                if *c < 3 {
                                    *c += 1;
                                    todo.push((sig.clone(), msg.clone(), *s, t.clone(), *c, errs.clone()));
                                }
                            }
                        }
                    }
                }
                for (sig, msg, s, t, n, errs) in todo {
                    let fdir = out.join("findings").join(&sig).join(n.to_string());
                    fs::create_dir_all(&fdir).unwrap();
                    fs::write(fdir.join("original.py"), t.source()).unwrap();
                    fs::write(fdir.join("tree.json"), serde_json::to_string(&t).unwrap()).unwrap();
                    let oracle = sig.split("__").next().unwrap_or(&sig).to_string();
                    fs::write(
                        fdir.join("finding.json"),
                        serde_json::to_string_pretty(&json!({
                            "signature": sig, "oracle": oracle, "message": msg, "seed": s,
                            "sig": sig, "msg": msg, "orig_size": t.size(), "errs": errs,
                        }))
                        .unwrap(),
                    )
                    .unwrap();
                    let repro = fdir.join("repro.sh");
                    fs::write(&repro, REPRO_SH).unwrap();
                    let _ = fs::set_permissions(&repro, std::os::unix::fs::PermissionsExt::from_mode(0o755));
                    eprintln!("[w{w}] finding {sig} (seed {s}, {} lines): {}", t.size(), msg.chars().take(200).collect::<String>());
                }
                let st = stats.lock().unwrap();
                let failures: u64 = st.fails.iter().filter(|(k, _)| !k.starts_with("known__")).map(|(_, n)| n).sum();
                let j = json!({
                    "cases": st.cases, "failures": failures, "invalid": st.invalid, "pass": st.pass, "probes": st.probes, "asserts": st.asserts,
                    "fails": st.fails, "invalid_kinds": st.invalid_kinds, "batches": st.batches, "ms_eval": st.ms_eval,
                });
                let _ = fs::write(out.join("stats.json"), serde_json::to_string_pretty(&j).unwrap());
            }
        }).unwrap());
    }
    for h in hs {
        let _ = h.join();
    }
    eprintln!("{}", fs::read_to_string(out.join("stats.json")).unwrap_or_default());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("search") => search(args[2].parse().unwrap(), args[3].parse().unwrap(), Path::new(&args[4])),
        Some("gen") => {
            let (seed, n, dir): (u64, usize, PathBuf) = (args[2].parse().unwrap(), args[3].parse().unwrap(), args[4].clone().into());
            fs::create_dir_all(&dir).unwrap();
            for i in 0..n {
                let mut src = Source::new(seed + i as u64, 30 + i * 70 / n.max(1));
                fs::write(dir.join(format!("p_{i}.py")), program_gen().generate(&mut src).source()).unwrap();
            }
        }
        Some("minimize") => {
            // tyz minimize <finding-dir> [secs]: min.py, min_check.py, and min_* in finding.json
            let fdir = PathBuf::from(&args[2]);
            let secs: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(150);
            let t: Tree = serde_json::from_str(&fs::read_to_string(fdir.join("tree.json")).unwrap()).unwrap();
            let mut f: Value = serde_json::from_str(&fs::read_to_string(fdir.join("finding.json")).unwrap()).unwrap();
            let sig = f["sig"].as_str().unwrap().to_string();
            let dir = std::env::temp_dir().join(format!("tyz-min-{}", std::process::id()));
            let alone = eval_batch(&dir.join("one"), &[t.source()]).pop().unwrap();
            let (ok, errs) = match &alone {
                Verdict::Fail { sig: s2, errs, .. } if *s2 == sig => (true, errs.clone()),
                _ => (false, vec![]),
            };
            f["reproduces_alone"] = json!(ok);
            if ok {
                let (min, mmsg) = minimize(&t, &sig, &errs, &dir.join("shrink"), secs);
                fs::write(fdir.join("min.py"), min.source()).unwrap();
                let _ = eval_batch(&dir.join("one"), &[min.source()]);
                fs::write(fdir.join("min_check.py"), fs::read_to_string(dir.join("one/check/c_0.py")).unwrap_or_default()).unwrap();
                f["min_msg"] = json!(mmsg);
                f["min_size"] = json!(min.size());
                f["errs"] = json!(errs);
            } else {
                f["alone"] = json!(format!("{alone:?}"));
            }
            fs::write(fdir.join("finding.json"), serde_json::to_string_pretty(&f).unwrap()).unwrap();
            let _ = fs::remove_dir_all(&dir);
            println!("{} {} -> {}", sig, f["orig_size"], f["min_size"]);
        }
        Some("repro") => {
            // tyz repro <finding-dir> <build-dir>: see REPRO_SH.
            let (fdir, build) = (PathBuf::from(&args[2]), PathBuf::from(&args[3]));
            let f: Value = serde_json::from_str(&fs::read_to_string(fdir.join("finding.json")).unwrap()).unwrap();
            let sig = f["signature"].as_str().or(f["sig"].as_str()).unwrap().to_string();
            let src = fs::read_to_string(fdir.join("original.py")).unwrap();
            // SAFETY: single-threaded here; eval_batch reads these.
            unsafe {
                std::env::set_var("TYZ_TY", build.join("bin/ty"));
                std::env::set_var("TYZ_PY", build.join("bin/py"));
            }
            let dir = std::env::temp_dir().join(format!("tyz-repro-{}", std::process::id()));
            let v = eval_batch(&dir, &[src]).pop().unwrap();
            let _ = fs::remove_dir_all(&dir);
            println!("{v:?}");
            std::process::exit(match v {
                Verdict::Fail { sig: s, .. } if s == sig => 1,
                Verdict::Invalid(_) => 2,
                _ => 0,
            });
        }
        Some("eval") => {
            let src = fs::read_to_string(&args[2]).unwrap();
            let dir = std::env::temp_dir().join(format!("tyz-eval-{}", std::process::id()));
            let v = eval_batch(&dir, &[src]).pop().unwrap();
            let c = fs::read_to_string(dir.join("check/c_0.py")).unwrap_or_default();
            let _ = std::io::stdout().write_all(c.as_bytes());
            println!("# verdict: {v:?}");
            std::process::exit(if matches!(v, Verdict::Fail { .. }) { 1 } else { 0 });
        }
        _ => eprintln!("usage: tyz search <seed> <secs> <out> | gen <seed> <n> <dir> | eval <file.py> | minimize <finding-dir> [secs] | repro <finding-dir> <build-dir>"),
    }
}
