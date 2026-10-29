// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! tyz: generated typed Python programs against ty (README.md).
//!
//!   tyz search <seed> <secs> <out>   batches of programs; findings under <out>/findings
//!   tyz gen <seed> <n> <dir>         write n programs (p_<i>.py) for inspection
//!   tyz eval <file.py>               one program: prints the verdict, exit 1 when it fails
//!
//! Environment: TYZ_TY (ty binary), TYZ_PY (python 3.13+), TYZ_WORKERS, TYZ_BATCH.

mod hint;
mod incr;
mod lsp;
mod pygen;
mod ren;
mod rt;
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
const META: &str = include_str!("../py/meta.py");
const ORD: &str = include_str!("../py/ord.py");

pub(crate) fn ty_bin() -> String {
    std::env::var("TYZ_TY").unwrap_or_else(|_| "ty".into())
}
pub(crate) fn py_bin() -> String {
    std::env::var("TYZ_PY").unwrap_or_else(|_| "python3".into())
}

/// Programs with pydantic models need the build's virtual environment (`<build>/venv`, where
/// build.sh installs pydantic and `bin/py` runs its python): ty finds it through VIRTUAL_ENV,
/// which every ty and tyinc run inherits. Call before any thread starts.
fn use_venv() {
    if std::env::var_os("VIRTUAL_ENV").is_some() {
        return;
    }
    let Some(build) = std::env::var_os("TYZ_PY").map(PathBuf::from).and_then(|p| p.parent()?.parent().map(Path::to_path_buf)) else { return };
    let venv = build.join("venv");
    if venv.join("pyvenv.cfg").is_file() {
        // SAFETY: called from main before any other thread exists.
        unsafe { std::env::set_var("VIRTUAL_ENV", &venv) };
    }
}

#[derive(Clone, Debug)]
enum Verdict {
    /// The program does not run cleanly under CPython (a generator miss).
    Invalid(String),
    Pass { probes: usize, asserts: usize },
    /// `errs`: the error diagnostics on program lines, as `rule: message`.
    /// `files`: extra files for the finding directory (the `incr__` oracle's steps).
    Fail { sig: String, msg: String, errs: Vec<String>, files: Vec<(String, String)> },
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
            // (The incr oracle's edits may cut a probe line short.)
            let Some((k, e)) = rest.split_once(", ") else { continue };
            let Some(e) = e.strip_suffix(')') else { continue };
            if let Some(tys) = obs.get(k) {
                if !tys.is_empty() && !tys.iter().any(|t| t == "?") {
                    let u = if tys.len() == 1 { tys[0].clone() } else { format!("_t.Union[{}]", tys.join(", ")) };
                    // Disjointness fails only if ty rules out the runtime value itself
                    // (`u` contains it however coarse it is); assignability also fails
                    // when ty cannot prove a coarse `u` fits, so it needs triage.
                    push(&mut out, &format!("{ind}static_assert(not is_disjoint_from({u}, TypeOf[{e}]))  # tyz:{k}:d"), true, &mut n);
                    // `Literal[True] | Literal[False]` collapses to `bool`, and ty does not split a
                    // `bool` source against a union target (its is_subtype_of.md: "TODO: No errors
                    // ... is_subtype_of(bool, Literal[True] | AlwaysFalsy)"), so `bool` fails against
                    // e.g. `Literal[True] | (int & ~AlwaysTruthy)` although both literals fit. A union
                    // is assignable iff each member is: check `Literal[False]` on its own.
                    let (t, f) = ("_t.Literal[True]", "_t.Literal[False]");
                    let mut parts: Vec<&String> = tys.iter().collect();
                    if tys.iter().any(|x| x == t) && tys.iter().any(|x| x == f) {
                        parts.retain(|x| *x != f);
                        push(&mut out, &format!("{ind}static_assert(is_assignable_to({f}, TypeOf[{e}]))  # tyz:{k}:a"), true, &mut n);
                    }
                    let ua = if parts.len() == 1 { parts[0].clone() } else { format!("_t.Union[{}]", parts.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")) };
                    push(&mut out, &format!("{ind}static_assert(is_assignable_to({ua}, TypeOf[{e}]))  # tyz:{k}:a"), true, &mut n);
                    asserts += 1;
                }
            }
        }
    }
    (out, inserted, asserts)
}

struct Diag {
    line: usize,
    col: usize,
    severity: String,
    rule: String,
    msg: String,
}

fn parse_diags(out: &str) -> BTreeMap<String, Vec<Diag>> {
    let mut by_file: BTreeMap<String, Vec<Diag>> = BTreeMap::new();
    for l in out.lines() {
        // path:line:col: severity[rule] message
        let mut it = l.splitn(4, ':');
        let (Some(path), Some(line), Some(col), Some(rest)) = (it.next(), it.next(), it.next(), it.next()) else { continue };
        let Ok(line) = line.parse::<usize>() else { continue };
        let col = col.parse::<usize>().unwrap_or(0);
        let rest = rest.trim_start();
        let Some((sev, rest)) = rest.split_once('[') else { continue };
        let Some((rule, msg)) = rest.split_once("] ") else { continue };
        let file = diag_key(Path::new(path));
        by_file.entry(file).or_default().push(Diag { line, col, severity: sev.to_string(), rule: rule.to_string(), msg: msg.to_string() });
    }
    by_file
}

/// The file a diagnostic is attributed to: its name, or `<package>/__init__.py`.
fn diag_key(p: &Path) -> String {
    let name = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    match p.parent().and_then(|d| d.file_name()) {
        Some(d) if name == "__init__.py" || name == "__init__.pyi" => format!("{}/{name}", d.to_string_lossy()),
        _ => name,
    }
}

fn panic_sig(msg: &str) -> String {
    // "Panicked at crates/x/src/y.rs:12:5 when checking ..."
    let loc = msg.strip_prefix("Panicked at ").and_then(|r| r.split_whitespace().next()).unwrap_or("unknown");
    let mut parts: Vec<&str> = loc.rsplitn(3, ':').collect();
    parts.reverse();
    let file = parts.first().copied().unwrap_or("unknown");
    let line = parts.get(1).copied().unwrap_or("0");
    let short = file.rsplit('/').next().unwrap_or(file).trim_end_matches(".rs");
    // Salsa's own assertions (backdate violations, cycle checks) fire in one place for every
    // query; the query's name tells the causes apart.
    if let Some((_, rest)) = msg.split_once("`query ")
        && let Some((query, _)) = rest.split_once('(')
        && query.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return format!("crash__panic__{short}_{line}_{query}");
    }
    format!("crash__panic__{short}_{line}")
}

/// `ty check` on a directory: its stdout, and a batch failure (hang, crash exit) if any.
fn run_ty(cdir: &Path, timeout: Duration) -> (String, Option<(String, String)>) {
    run_ty_on(cdir, timeout, &[], None)
}

/// `run_ty` on some of the directory's files only (none: all), with TY_MAX_PARALLELISM `threads`.
fn run_ty_on(cdir: &Path, timeout: Duration, paths: &[String], threads: Option<usize>) -> (String, Option<(String, String)>) {
    let t0 = Instant::now();
    let mut cmd = Command::new(ty_bin());
    if let Some(n) = threads {
        cmd.env("TY_MAX_PARALLELISM", n.to_string());
    }
    let mut child = cmd
        .args(["check", "--python-version", "3.13", "--output-format", "concise", "--no-progress", "--project", "."])
        .args(paths)
        .current_dir(cdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ty");
    // Drain the pipes while ty runs (a large batch's output can exceed the pipe buffer).
    use std::io::Read;
    let mut o = child.stdout.take().unwrap();
    let mut e = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = o.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = e.read_to_string(&mut s);
        s
    });
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
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();
    let fail = match status {
        None => Some(("crash__hang__ty".to_string(), format!("ty ran past {timeout:?}"))),
        Some(s) if s.code().is_none_or(|c| c > 1) => Some((format!("crash__exit__ty_{}", s.code().map(|c| c.to_string()).unwrap_or("signal".into())), stderr.chars().take(2000).collect())),
        _ => None,
    };
    (stdout, fail)
}

/// The `meta__` oracle (py/meta.py) on the programs of a batch that passed the other oracles:
/// each check file with its annotations spelled differently (aliases, other spellings, strings)
/// must still have no error and no failing assertion. `meta__<variant>__<rule>`, where the rule is
/// `assign`/`disjoint` for an assertion that held in the original.
fn meta_batch(dir: &Path, cdir: &Path, verdicts: &mut [Verdict]) {
    let ok: Vec<usize> = (0..verdicts.len()).filter(|&i| matches!(verdicts[i], Verdict::Pass { .. })).collect();
    if ok.is_empty() {
        return;
    }
    let mdir = dir.join("meta");
    let _ = fs::remove_dir_all(&mdir);
    fs::create_dir_all(&mdir).unwrap();
    fs::write(mdir.join("_rt.py"), "def _probe(k: int, v: object) -> None: ...\n").unwrap();
    fs::write(dir.join("meta.py"), META).unwrap();
    // A search batch checks the negate, module and stub variants and one of the others per program
    // (by position); a single program (repro, minimize) gets all six.
    let names: Vec<String> =
        ok.iter().map(|&i| if verdicts.len() == 1 { format!("c_{i}.py") } else { format!("c_{i}.py:nmp{}", &"asq"[i % 3..i % 3 + 1]) }).collect();
    let _ = Command::new(py_bin()).arg("-I").arg(dir.join("meta.py")).arg(cdir).arg(&mdir).args(&names).stderr(Stdio::null()).status();
    let (stdout, batch_fail) = run_ty(&mdir, Duration::from_secs(60 + 4 * ok.len() as u64));
    let diags = parse_diags(&stdout);
    let variant = |v: &str| match v {
        "n" => "negate",
        "a" => "alias",
        "s" => "spell",
        _ => "quote",
    };
    for &i in &ok {
        for v in ["n", "a", "s", "q", "m", "p"] {
            let f = format!("{v}_{i}.py");
            let Ok(src) = fs::read_to_string(mdir.join(&f)) else { continue };
            let mut files = vec![(format!("meta/{f}"), src.clone())];
            // The module variant: the other files it imports, and which way.
            let mut vname = variant(v).to_string();
            let mut parts = vec![(f.clone(), src.clone())];
            if v == "m" || v == "p" {
                let info: Value = fs::read_to_string(mdir.join(format!("{v}_{i}.json"))).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
                vname = match info["style"].as_str().unwrap_or("?") {
                    // The stub variant (py/meta.py `p`): one signature per rule, `stub-pyi` when the
                    // diagnostic is in a generated `.pyi`.
                    _ if v == "p" => "stub".to_string(),
                    s if s.starts_with("tailstar") => "module-tailstar".to_string(),
                    s => format!("module-{s}"),
                };
                let listed: Vec<&str> = info["files"].as_array().into_iter().flatten().filter_map(|p| p.as_str()).collect();
                for &p in &listed {
                    let t = fs::read_to_string(mdir.join(p)).unwrap_or_default();
                    files.push((format!("meta/{p}"), t.clone()));
                    // A module's own `.py` next to its stub: when it imports names back, its classes are
                    // not the stub's for ty (two definitions), so only importers and stubs count.
                    if !listed.contains(&format!("{p}i").as_str()) {
                        parts.push((diag_key(Path::new(p)), t));
                    }
                }
            }
            if let Some((sig, msg)) = &batch_fail {
                // Only the original check passed, so the variants' check broke it.
                verdicts[i] = Verdict::Fail { sig: format!("{sig}_meta"), msg: msg.clone(), errs: vec![], files };
                break;
            }
            let empty = Vec::new();
            let mut failed = false;
            for (pf, psrc) in &parts {
                let ds = diags.get(pf).unwrap_or(&empty);
                let lines: Vec<&str> = psrc.lines().collect();
                let line_of = |d: &Diag| lines.get(d.line - 1).copied().unwrap_or("").trim().to_string();
                let hit = ds.iter().find(|d| d.rule == "panic" || d.severity == "fatal").map(|d| (panic_sig(&d.msg), d)).or_else(|| {
                    ds.iter().find(|d| d.severity == "error").map(|d| {
                        let rule = match d.rule.as_str() {
                            "static-assert-error" if line_of(d).ends_with(":d") => "disjoint",
                            "static-assert-error" => "assign",
                            r => r,
                        };
                        // `Divergent` (a cycle's placeholder) in any message: a leaked cycle.
                        // The module variant's rare mutual-`*` shape (py/meta.py) has one bug of its own.
                        let vname = if vname == "stub" && pf.ends_with(".pyi") { "stub-pyi" } else { vname.as_str() };
                        let rule = if vname.starts_with("module-tailstar") {
                            "star-cycle"
                        } else if ds.iter().any(|d| d.severity == "error" && d.msg.contains("Divergent")) {
                            "divergent"
                        } else {
                            rule
                        };
                        (format!("meta__{vname}__{rule}"), d)
                    })
                });
                if let Some((sig, d)) = hit {
                    let errs = ds.iter().filter(|d| d.severity == "error").map(|d| format!("{}: {}", d.rule, d.msg)).collect();
                    verdicts[i] = Verdict::Fail { sig, msg: format!("{pf} line {}: {} ({})", d.line, d.msg, line_of(d)), errs, files: files.clone() };
                    failed = true;
                    break;
                }
            }
            if failed {
                break;
            }
        }
    }
}

/// The kind of runtime value a `:d` assertion line describes (`static_assert(not
/// is_disjoint_from(<U>, TypeOf[e]))`): `func` (a function or class object, `TypeOf[f]`),
/// `literal`, `union`, `tuple`, `float`, `none`, the builtin class of an instance (`list`, ...),
/// or `instance` (a program class).
fn value_kind(line: &str) -> String {
    let u = line.split_once("is_disjoint_from(").map_or("", |(_, r)| r);
    let u = u.strip_prefix("_tx.Intersection[").unwrap_or(u);
    let head: String = u.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.').collect();
    let head = head.rsplit('.').next().unwrap_or("").to_string();
    match head.as_str() {
        "TypeOf" if u.starts_with("TypeOf[0.") || u.starts_with("TypeOf[-") || u.contains("TypeOf[float") => "float".into(),
        "TypeOf" => "func".into(),
        "Literal" | "LiteralString" => "literal".into(),
        "Union" | "Optional" => "union".into(),
        "None" | "NoneType" => "none".into(),
        h if h.chars().next().is_some_and(|c| c.is_ascii_lowercase()) => h.into(),
        _ => "instance".into(),
    }
}

/// Whether the value a `:d` assertion at 1-based `line` checks is a target of an enclosing `for`
/// over `enumerate(...)`/`zip(...)` of a conditional expression: one known bug (an `Unknown`
/// iterable in the union is dropped when their type variable is solved), `ret__disjoint__zipenum`.
fn zip_enum_union(lines: &[&str], line: usize) -> bool {
    let Some(probe) = line.checked_sub(2).and_then(|i| lines.get(i)) else { return false };
    let Some(var) = probe.trim().strip_prefix("_probe(").and_then(|r| r.split_once(", ")).map(|(_, v)| v.trim_end_matches(')').to_string()) else { return false };
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut cur = indent(probe);
    for l in lines[..line - 2].iter().rev() {
        if l.trim().is_empty() || indent(l) >= cur {
            continue;
        }
        cur = indent(l);
        let t = l.trim();
        if let Some((targets, iter)) = t.strip_prefix("for ").and_then(|r| r.split_once(" in ")) {
            if targets.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == var) {
                return (iter.starts_with("enumerate(") || iter.starts_with("zip(")) && iter.contains(" if ");
            }
        }
        if cur == 0 {
            break;
        }
    }
    false
}

/// The `ret__`/`ord__` oracle (py/ord.py) on the programs of a batch that passed the other oracles,
/// after meta_batch (it reuses the module splits): each check file and its module split with most
/// return annotations dropped, under `return-type-inference = "always"`, each module revealing its
/// top-level names, plus a module importing the single-file version and revealing them again.
/// - `ret__disjoint__<kind>[-x]`: an inferred return type excludes a value the function returned at
///   runtime (a `:d` assertion that held with the annotations fails); `<kind>`: `value_kind`; `-x`
///   when it fails only in the module split (across modules);
/// - `ord__<run>-<rule>__<unit>`: checking only the importing modules (`entry`), only the imported
///   ones (`defs`), or all of them on one thread (`seq`) gives a checked file other diagnostics
///   (reveals included) than checking all of them in parallel: the result depends on which query
///   enters first. `<rule>` is the first differing diagnostic's (`divergent` when one names
///   `Divergent`); `<unit>` is `single` or `module-<head|tail|headcycle>`.
/// The three order runs happen in one search batch of TYZ_ORD_EVERY (2); a single program always
/// gets them. Other errors are not findings: inferred returns are narrower than the dropped annotations
/// (a dict literal for a TypedDict, invariant containers), which ty documents.
fn ord_batch(dir: &Path, cdir: &Path, srcs: &[String], verdicts: &mut [Verdict]) {
    let ok: Vec<usize> = (0..verdicts.len()).filter(|&i| matches!(verdicts[i], Verdict::Pass { .. })).collect();
    if ok.is_empty() {
        return;
    }
    let odir = dir.join("ord");
    let _ = fs::remove_dir_all(&odir);
    fs::write(dir.join("ord.py"), ORD).unwrap();
    let out = Command::new(py_bin())
        .arg("-I")
        .arg(dir.join("ord.py"))
        .arg(cdir)
        .arg(dir.join("meta"))
        .arg(&odir)
        .args(ok.iter().map(|i| i.to_string()))
        .stderr(Stdio::null())
        .output()
        .expect("python");
    let recs: Vec<Value> = String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    if recs.is_empty() {
        return;
    }
    let strs = |v: &Value| -> Vec<String> { v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect() };
    let entry: Vec<String> = recs.iter().flat_map(|r| strs(&r["entry"])).collect();
    let defs: Vec<String> = recs.iter().flat_map(|r| strs(&r["defs"])).collect();
    let timeout = Duration::from_secs(60 + 4 * ok.len() as u64);
    let (full_out, full_fail) = run_ty_on(&odir, timeout, &[], None);
    let full = parse_diags(&full_out);
    let mut runs = vec![];
    let every = std::env::var("TYZ_ORD_EVERY").ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(2);
    if full_fail.is_none() && (srcs.len() == 1 || incr::hash(&srcs[0]) % every.max(1) == 0) {
        for (name, paths, threads) in [("entry", &entry, None), ("defs", &defs, None), ("seq", &vec![], Some(1))] {
            let (o, f) = run_ty_on(&odir, timeout, paths, threads);
            runs.push((name, parse_diags(&o), f, paths.clone()));
        }
    }
    let key = |d: &Diag| format!("{}:{}[{}] {}", d.line, d.severity, d.rule, d.msg);
    let empty = Vec::new();
    for r in &recs {
        let Some(i) = r["i"].as_u64().map(|i| i as usize) else { continue };
        let mut units: Vec<(String, Vec<String>)> = r["units"].as_object().into_iter().flatten().map(|(k, v)| (k.clone(), strs(v))).collect();
        // The single-file unit first: a bug that shows there too gets its signature.
        units.sort_by_key(|(u, _)| u != "single");
        let mut files = vec![];
        for (_, fs_) in &units {
            for f in fs_ {
                files.push((format!("ord/{f}"), fs::read_to_string(odir.join(f)).unwrap_or_default()));
            }
        }
        for f in ["ty.toml", "_rt.py"] {
            files.push((format!("ord/{f}"), fs::read_to_string(odir.join(f)).unwrap_or_default()));
        }
        let mut hit: Option<(String, String)> = None;
        'units: for (unit, fs_) in &units {
            if let Some((sig, msg)) = &full_fail {
                hit = Some((format!("{sig}_ord"), msg.clone()));
                break;
            }
            // An error (an inferred return narrower than the annotation it replaces) can change
            // what the probes see through declarations: no assertion counts in such a unit.
            let clean = !fs_.iter().any(|f| full.get(&diag_key(Path::new(f))).unwrap_or(&empty).iter().any(|d| d.severity == "error" && d.rule != "static-assert-error"));
            for f in fs_ {
                let k = diag_key(Path::new(f));
                let src = fs::read_to_string(odir.join(f)).unwrap_or_default();
                let lines: Vec<&str> = src.lines().collect();
                let line_of = |d: &Diag| lines.get(d.line - 1).copied().unwrap_or("").trim().to_string();
                let ds = full.get(&k).unwrap_or(&empty);
                if let Some(d) = ds.iter().find(|d| d.rule == "panic" || d.severity == "fatal") {
                    hit = Some((panic_sig(&d.msg), format!("{f}: {}", d.msg)));
                    break 'units;
                }
                if let Some(d) = ds.iter().find(|d| clean && d.rule == "static-assert-error" && line_of(d).ends_with(":d")) {
                    let x = if unit == "single" { "" } else { "-x" };
                    let kind = if zip_enum_union(&lines, d.line) { "zipenum".to_string() } else { value_kind(&line_of(d)) };
                    hit = Some((format!("ret__disjoint__{kind}{x}"), format!("{f} line {}: {}", d.line, line_of(d))));
                    break 'units;
                }
                for (run, rd, rfail, paths) in &runs {
                    if let Some((sig, msg)) = rfail {
                        hit = Some((format!("{sig}_ord"), msg.clone()));
                        break 'units;
                    }
                    if !paths.is_empty() && !paths.contains(f) {
                        continue;
                    }
                    let rds = rd.get(&k).unwrap_or(&empty);
                    if let Some(d) = rds.iter().find(|d| d.rule == "panic" || d.severity == "fatal") {
                        hit = Some((panic_sig(&d.msg), format!("{f} ({run}): {}", d.msg)));
                        break 'units;
                    }
                    let mut a: Vec<String> = ds.iter().map(key).collect();
                    let mut b: Vec<String> = rds.iter().map(key).collect();
                    a.sort();
                    b.sort();
                    if a != b {
                        let only_a: Vec<&String> = a.iter().filter(|x| !b.contains(x)).collect();
                        let only_b: Vec<&String> = b.iter().filter(|x| !a.contains(x)).collect();
                        let first = ds.iter().chain(rds.iter()).find(|d| {
                            let kd = key(d);
                            only_a.contains(&&kd) || only_b.contains(&&kd)
                        });
                        let rule = match first {
                            _ if only_a.iter().chain(only_b.iter()).any(|x| x.contains("Divergent")) => "divergent".to_string(),
                            Some(d) => d.rule.clone(),
                            None => "count".to_string(),
                        };
                        let msg = format!("{f}: all files in parallel only {:?}; {run} only {:?}", only_a.iter().take(3).collect::<Vec<_>>(), only_b.iter().take(3).collect::<Vec<_>>());
                        hit = Some((format!("ord__{run}-{rule}__{unit}"), msg));
                        break 'units;
                    }
                }
            }
        }
        if let Some((sig, msg)) = hit {
            verdicts[i] = Verdict::Fail { sig, msg, errs: vec![], files };
        }
    }
}

/// The truthiness oracle: ty's redundant-condition(-strict) warning claims that a condition is
/// always truthy or always falsy ("Detects boolean conditions where the condition can be
/// statically inferred to be always true or always false"); CPython observed the other truth
/// value for it. `truth`: per condition (`line:col:end_line:end_col` in the program), the truth
/// values observed, and its construct. Several conditions can start where a diagnostic does
/// (`a and b` and `a`): unless the message's expression text singles one out, each of them
/// must have run and contradict the claim.
/// ty's ranges include parentheses, CPython's do not: the conditions starting after the
/// warning's `(`s count too.
fn truth_verdict(ds: &[Diag], ins: &BTreeSet<usize>, lines: &[&str], truth: &(BTreeMap<String, Vec<bool>>, BTreeMap<String, String>)) -> Option<(String, String)> {
    let (seen, kind) = truth;
    for d in ds {
        if d.rule != "redundant-condition" && d.rule != "redundant-condition-strict" || ins.contains(&d.line) {
            continue;
        }
        let claim = if d.msg.contains("always truthy") || d.msg.contains("always true") {
            true
        } else if d.msg.contains("always falsy") || d.msg.contains("always false") {
            false
        } else {
            continue;
        };
        let line = d.line - ins.range(..d.line).count();
        let text: Vec<char> = lines.get(d.line - 1).copied().unwrap_or("").chars().collect();
        let mut ats = vec![format!("{line}:{}:", d.col)];
        let mut c = d.col;
        while c >= 1 && text.get(c - 1) == Some(&'(') {
            c += 1;
            while text.get(c - 1) == Some(&' ') {
                c += 1;
            }
            ats.push(format!("{line}:{c}:"));
        }
        let mut cands: Vec<&String> = kind.keys().filter(|k| ats.iter().any(|at| k.starts_with(at))).collect();
        // "Condition `e` is ...": the one whose source is `e` (when its length tells).
        if let Some((_, rest)) = d.msg.split_once("Condition `")
            && let Some((e, _)) = rest.rsplit_once("` is always")
        {
            let n = e.chars().count();
            let fit: Vec<&String> = cands
                .iter()
                .copied()
                .filter(|k| {
                    let v: Vec<usize> = k.split(':').map(|x| x.parse().unwrap_or(0)).collect();
                    v.len() == 4 && v[0] == v[2] && v[3] == v[1] + n
                })
                .collect();
            if !fit.is_empty() {
                cands = fit;
            }
        }
        let ran: Vec<(&String, &Vec<bool>)> = cands.iter().filter_map(|k| seen.get(*k).map(|v| (*k, v))).collect();
        // A candidate that never ran may be the one the claim is about.
        if cands.is_empty() || ran.len() < cands.len() || !ran.iter().all(|(_, v)| v.contains(&!claim)) {
            continue;
        }
        // The outermost condition starting there names the construct.
        let k = ran.iter().map(|(k, _)| *k).max_by_key(|k| k.split(':').skip(2).map(|x| x.parse::<usize>().unwrap_or(0)).collect::<Vec<_>>()).unwrap();
        let construct = kind.get(k).map_or("cond", |s| s.as_str());
        let obs = if claim { "falsy" } else { "truthy" };
        return Some((format!("truth__{}__{construct}", d.rule), format!("line {line}: {} (CPython: {obs})", d.msg)));
    }
    None
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
    let mut obs_all: Vec<BTreeMap<String, Vec<String>>> = vec![BTreeMap::new(); srcs.len()];
    let mut truth_all: Vec<(BTreeMap<String, Vec<bool>>, BTreeMap<String, String>)> = vec![Default::default(); srcs.len()];
    // The truthiness oracle's claims: ty's redundant-condition rules, the strict one included.
    fs::write(cdir.join("ty.toml"), "[rules]\nredundant-condition-strict = \"warn\"\n").unwrap();
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
        obs_all[i] = obs;
        truth_all[i] = (serde_json::from_value(r["truth"].clone()).unwrap_or_default(), serde_json::from_value(r["kind"].clone()).unwrap_or_default());
        any = true;
    }
    if !any {
        return verdicts.into_iter().map(|v| v.unwrap()).collect();
    }
    let timeout = Duration::from_secs(60 + srcs.len() as u64);
    let (stdout, batch_fail) = run_ty(&cdir, timeout);
    let diags = parse_diags(&stdout);
    for i in 0..srcs.len() {
        if verdicts[i].is_some() {
            continue;
        }
        if let Some((sig, msg)) = &batch_fail {
            // Attributed to every program of the batch; the shrinker isolates it.
            verdicts[i] = Some(Verdict::Fail { sig: sig.clone(), msg: msg.clone(), errs: vec![], files: vec![] });
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
            Verdict::Fail { sig: panic_sig(&d.msg), msg: d.msg.clone(), errs: errs2, files: vec![] }
        } else if let Some(d) = base_err {
            Verdict::Fail { sig: format!("fp__{}", d.rule), msg: format!("line {}: {}", d.line, d.msg), errs: errs2, files: vec![] }
        } else if let Some(d) = ins_other {
            Verdict::Fail { sig: format!("harness__assert__{}", d.rule), msg: format!("line {}: {}", d.line, d.msg), errs: errs2, files: vec![] }
        } else if let Some(d) = sound_d {
            Verdict::Fail { sig: "sound__disjoint".into(), msg: format!("line {}: {}", d.line, line_of(d)), errs: errs2, files: vec![] }
        } else if let Some(d) = sound {
            Verdict::Fail { sig: "sound__assign".into(), msg: format!("line {}: {}", d.line, line_of(d)), errs: errs2, files: vec![] }
        } else if let Some((sig, msg)) = truth_verdict(ds, ins, &lines, &truth_all[i]) {
            Verdict::Fail { sig, msg, errs: errs2, files: vec![] }
        } else {
            Verdict::Pass { probes: *probes, asserts: *asserts }
        });
    }
    let mut verdicts: Vec<Verdict> = verdicts.into_iter().map(|v| v.unwrap()).collect();
    if std::env::var("TYZ_META").map_or(true, |v| v != "0") {
        meta_batch(dir, &cdir, &mut verdicts);
    }
    if std::env::var("TYZ_ORD").map_or(true, |v| v != "0") {
        ord_batch(dir, &cdir, srcs, &mut verdicts);
    }
    if incr::enabled() {
        incr_batch(dir, srcs, &obs_all, &mut verdicts);
    }
    if rt::enabled() {
        rt_batch(dir, srcs, &mut verdicts);
    }
    if ren::enabled() {
        ren_batch(dir, srcs, &obs_all, &mut verdicts);
    }
    if hint::enabled() {
        hint_batch(dir, srcs, &obs_all, &mut verdicts);
    }
    if lsp::enabled() {
        lsp_batch(dir, &cdir, srcs, &mut verdicts);
    }
    verdicts
}

/// The `lsp__` oracle (src/lsp.rs, py/lsp.py) on the check files of the first TYZ_LSP_N (6)
/// programs that passed the other oracles, in one search batch of TYZ_LSP_EVERY (2): one `ty server`
/// session. A finding keeps the program alone when it shows alone, the session's files otherwise.
fn lsp_batch(dir: &Path, cdir: &Path, srcs: &[String], verdicts: &mut [Verdict]) {
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
    let (n, every) = (env("TYZ_LSP_N", 6), env("TYZ_LSP_EVERY", 2) as u64);
    let seed = incr::hash(&srcs[0]).rotate_left(29);
    if srcs.len() > 1 && seed % every.max(1) != 0 {
        return;
    }
    let ok: Vec<usize> = (0..srcs.len()).filter(|&i| matches!(verdicts[i], Verdict::Pass { .. })).take(n).collect();
    let name = |i: usize| format!("c_{i}.py");
    let files: BTreeMap<String, String> =
        ok.iter().filter_map(|&i| fs::read_to_string(cdir.join(name(i))).ok().map(|t| (name(i), t))).collect();
    if files.is_empty() {
        return;
    }
    let prog = |f: &str| f.trim_start_matches("c_").trim_end_matches(".py").parse::<usize>().ok();
    let found = lsp::check(&dir.join("lsp"), &files, seed, None, panic_sig);
    let mut done = BTreeSet::new();
    for (p, sig, msg) in found {
        let i = p.as_deref().and_then(prog).unwrap_or(ok[0]);
        if !done.insert(i) {
            continue;
        }
        let f = name(i);
        let one: BTreeMap<String, String> = files.get(&f).map(|t| [(f.clone(), t.clone())].into()).unwrap_or_default();
        let force = lsp::force_for(&sig);
        let alone = !one.is_empty() && lsp::check(&dir.join("lsp-one"), &one, seed, Some(&force), panic_sig).iter().any(|(_, s, _)| *s == sig);
        let keep = if alone { &one } else { &files };
        verdicts[i] = Verdict::Fail { sig, msg, errs: vec![], files: lsp::finding_files(keep, seed) };
    }
}

/// The `hint__` oracle (src/hint.rs) on the first TYZ_HINT_N (16) programs of a batch that passed
/// the other oracles: one `tyinc --hints`, one CPython run and one `ty check` per batch.
fn hint_batch(dir: &Path, srcs: &[String], obs: &[BTreeMap<String, Vec<String>>], verdicts: &mut [Verdict]) {
    let n = std::env::var("TYZ_HINT_N").ok().and_then(|s| s.parse().ok()).unwrap_or(16);
    let force = std::env::var("TYZ_HINT_FORCE").ok();
    let ok: Vec<usize> = (0..srcs.len()).filter(|&i| matches!(verdicts[i], Verdict::Pass { .. })).take(n).collect();
    if ok.is_empty() {
        return;
    }
    let name = |i: usize| format!("p_{i}.py");
    let all: BTreeMap<String, String> = ok.iter().map(|&i| (name(i), srcs[i].clone())).collect();
    let obs_v: BTreeMap<String, Value> = ok.iter().map(|&i| (name(i), json!(obs[i]))).collect();
    let prog = |f: &str| f.trim_start_matches("p_").trim_end_matches(".py").parse::<usize>().unwrap();
    match hint::check(&dir.join("hint"), &all, &obs_v, force.as_deref(), panic_sig) {
        Err((sig, msg)) => {
            // A tyinc panic or hang: attributed to the first program; the one that shows it alone if any.
            let mut at = ok[0];
            for &i in &ok {
                let one: BTreeMap<String, String> = [(name(i), srcs[i].clone())].into();
                if let Err((s, _)) = hint::check(&dir.join("hint-one"), &one, &obs_v, force.as_deref(), panic_sig)
                    && s == sig
                {
                    at = i;
                    break;
                }
            }
            verdicts[at] = Verdict::Fail { sig, msg, errs: vec![], files: hint::finding_files(&name(at), &srcs[at], &obs_v[&name(at)]) };
        }
        Ok(found) => {
            for (f, (sig, msg)) in found {
                let i = prog(&f);
                verdicts[i] = Verdict::Fail { sig, msg, errs: vec![], files: hint::finding_files(&f, &srcs[i], &obs_v[&f]) };
            }
        }
    }
}

/// The `ren__` oracle (src/ren.rs) on the programs of a batch that passed the other oracles.
fn ren_batch(dir: &Path, srcs: &[String], obs: &[BTreeMap<String, Vec<String>>], verdicts: &mut [Verdict]) {
    // TYZ_REN_K (3) renames each of the first TYZ_REN_N (16) passing programs, in one search batch
    // of TYZ_REN_EVERY (2). A single program (repro, minimize, eval) always gets them.
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
    let (n, k, every) = (env("TYZ_REN_N", 16), env("TYZ_REN_K", 3), env("TYZ_REN_EVERY", 2) as u64);
    if srcs.len() > 1 && incr::hash(&srcs[0]).rotate_left(17) % every.max(1) != 0 {
        return;
    }
    let ok: Vec<usize> = (0..srcs.len()).filter(|&i| matches!(verdicts[i], Verdict::Pass { .. })).take(n).collect();
    if ok.is_empty() {
        return;
    }
    let name = |i: usize| format!("p_{i}.py");
    let all: BTreeMap<String, String> = ok.iter().map(|&i| (name(i), srcs[i].clone())).collect();
    let obs_v: BTreeMap<String, Value> = ok.iter().map(|&i| (name(i), json!(obs[i]))).collect();
    let reqs: Vec<ren::Req> = ok.iter().flat_map(|&i| ren::requests(&name(i), &srcs[i], k)).collect();
    let prog = |r: &ren::Req| r.file.trim_start_matches("p_").trim_end_matches(".py").parse::<usize>().unwrap();
    let alone = |r: &ren::Req, sig: &str| -> Vec<(String, String)> {
        let i = prog(r);
        let one: BTreeMap<String, String> = [(name(i), srcs[i].clone())].into();
        let obs1: BTreeMap<String, Value> = [(name(i), obs_v[&name(i)].clone())].into();
        let hit = match ren::check(&dir.join("ren-one"), &one, std::slice::from_ref(r), &obs1, panic_sig) {
            Err((s, _)) => s == sig,
            Ok(f) => f.values().any(|(s, _)| s == sig),
        };
        if hit { ren::finding_files(&one, r, &obs1) } else { ren::finding_files(&all, r, &obs_v) }
    };
    // TYZ_REN_M (2) of them also split over modules (py/meta.py's `m` variant), renamed across files.
    let m = env("TYZ_REN_M", 2);
    let split: Vec<usize> = ok.iter().rev().take(m).copied().collect();
    if !split.is_empty() {
        let (msrc, mout) = (dir.join("ren-msrc"), dir.join("ren-mout"));
        let _ = fs::remove_dir_all(&msrc);
        let _ = fs::remove_dir_all(&mout);
        fs::create_dir_all(&msrc).unwrap();
        fs::create_dir_all(&mout).unwrap();
        fs::write(dir.join("meta.py"), META).unwrap();
        for &i in &split {
            fs::write(msrc.join(format!("c_{i}.py")), &srcs[i]).unwrap();
        }
        let names: Vec<String> = split.iter().map(|&i| format!("c_{i}.py:m")).collect();
        let _ = Command::new(py_bin()).arg("-I").arg(dir.join("meta.py")).arg(&msrc).arg(&mout).args(&names).stderr(Stdio::null()).status();
        for &i in &split {
            let Ok(j) = fs::read_to_string(mout.join(format!("m_{i}.json"))) else { continue };
            let j: Value = serde_json::from_str(&j).unwrap_or_default();
            // Mutual `*` imports leak `Divergent` (meta__module-tailstar__star-cycle): not renamed.
            if j["style"].as_str().is_none_or(|s| s.starts_with("tailstar")) {
                continue;
            }
            let mut unit = BTreeMap::new();
            for f in std::iter::once(format!("m_{i}.py")).chain(j["files"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string))) {
                if let Ok(t) = fs::read_to_string(mout.join(&f)) {
                    unit.insert(f, t);
                }
            }
            let entry = format!("e_{i}.py");
            unit.insert(entry.clone(), format!("import m_{i}\n"));
            let reqs = ren::unit_requests(&unit, &entry, k, None);
            let w = dir.join("ren-unit");
            let res = ren::check_unit(&w, &unit, &entry, &reqs, panic_sig);
            let hit = match res {
                Err(f) => reqs.first().map(|r| (r.clone(), f)),
                Ok(found) => found.into_iter().next().map(|(j, f)| (reqs[j].clone(), f)),
            };
            if let Some((r, (sig, msg))) = hit {
                let style = j["style"].as_str().unwrap_or("?");
                verdicts[i] = Verdict::Fail { sig, msg: format!("{msg} (split {style})"), errs: vec![], files: ren::unit_files(&unit, &entry, &r) };
            }
        }
    }
    // TYZ_REN_Q (2) others with their annotations quoted (py/meta.py's `q` variant): ty renames
    // inside string annotations (rename.rs `rename_string_annotation*`).
    let q: Vec<usize> = ok.iter().rev().skip(m).take(env("TYZ_REN_Q", 2)).copied().collect();
    if !q.is_empty() {
        let (msrc, mout) = (dir.join("ren-qsrc"), dir.join("ren-qout"));
        let _ = fs::remove_dir_all(&msrc);
        let _ = fs::remove_dir_all(&mout);
        fs::create_dir_all(&msrc).unwrap();
        fs::create_dir_all(&mout).unwrap();
        fs::write(dir.join("meta.py"), META).unwrap();
        for &i in &q {
            fs::write(msrc.join(format!("c_{i}.py")), &srcs[i]).unwrap();
        }
        let names: Vec<String> = q.iter().map(|&i| format!("c_{i}.py:q")).collect();
        let _ = Command::new(py_bin()).arg("-I").arg(dir.join("meta.py")).arg(&msrc).arg(&mout).args(&names).stderr(Stdio::null()).status();
        for &i in &q {
            let Ok(t) = fs::read_to_string(mout.join(format!("q_{i}.py"))) else { continue };
            let entry = format!("e_{i}.py");
            let unit: BTreeMap<String, String> = [(format!("q_{i}.py"), t), (entry.clone(), format!("import q_{i}\n"))].into();
            let reqs = ren::unit_requests(&unit, &entry, k, Some(&srcs[i]));
            let res = ren::check_unit(&dir.join("ren-unit"), &unit, &entry, &reqs, panic_sig);
            let hit = match res {
                Err(f) => reqs.first().map(|r| (r.clone(), f)),
                Ok(found) => found.into_iter().next().map(|(j, f)| (reqs[j].clone(), f)),
            };
            if let Some((r, (sig, msg))) = hit {
                verdicts[i] = Verdict::Fail { sig, msg, errs: vec![], files: ren::unit_files(&unit, &entry, &r) };
            }
        }
    }
    match ren::check(&dir.join("ren"), &all, &reqs, &obs_v, panic_sig) {
        Err((sig, msg)) => {
            let i = ok[0];
            let files = reqs.iter().find(|r| prog(r) == i).map(|r| alone(r, &sig)).unwrap_or_default();
            verdicts[i] = Verdict::Fail { sig, msg, errs: vec![], files };
        }
        Ok(found) => {
            let mut done = BTreeSet::new();
            for (j, (sig, msg)) in found {
                let i = prog(&reqs[j]);
                if !matches!(verdicts[i], Verdict::Pass { .. }) || !done.insert(i) {
                    continue;
                }
                let files = alone(&reqs[j], &sig);
                verdicts[i] = Verdict::Fail { sig, msg, errs: vec![], files };
            }
        }
    }
}

/// The `rt__` oracle (src/rt.rs) on the programs of a batch that passed the other oracles.
fn rt_batch(dir: &Path, srcs: &[String], verdicts: &mut [Verdict]) {
    // Eleven `ty check` runs per batch, on the first TYZ_RT_N (8) passing programs, in one search
    // batch of two (TYZ_RT_EVERY): the cost stays a fraction of the search's. A single program
    // (repro, minimize, eval) always gets them.
    let n: usize = std::env::var("TYZ_RT_N").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    let every: u64 = std::env::var("TYZ_RT_EVERY").ok().and_then(|s| s.parse().ok()).unwrap_or(2);
    if srcs.len() > 1 && incr::hash(&srcs[0]) % every.max(1) != 0 {
        return;
    }
    let ok: Vec<usize> = (0..srcs.len()).filter(|&i| matches!(verdicts[i], Verdict::Pass { .. })).take(n).collect();
    if ok.is_empty() {
        return;
    }
    let per: BTreeMap<usize, Vec<(String, String)>> = ok.iter().map(|&i| (i, rt::inputs(i, &srcs[i]))).collect();
    let all: Vec<(String, String)> = per.values().flatten().cloned().collect();
    let work = dir.join("rt");
    rt::write_inputs(&work, &all);
    let found = rt::check(&work, panic_sig);
    let finding_files = |files: &[(String, String)], only: &[String]| {
        let mut f: Vec<(String, String)> = files.iter().map(|(n, t)| (format!("rt/in/{n}"), t.clone())).collect();
        if !only.is_empty() {
            f.push(("rt/only.txt".into(), only.iter().map(|o| format!("{o}\n")).collect()));
        }
        f
    };
    let mut done = BTreeSet::new();
    for (f, (sig, msg)) in found {
        // A finding not tied to a file goes to the first program.
        let i = f.trim_end_matches(".py").split_once('_').and_then(|(_, n)| n.parse::<usize>().ok()).unwrap_or(ok[0]);
        if !per.contains_key(&i) || !done.insert(i) {
            continue;
        }
        // The program alone reproduces it, or else the whole batch is the repro.
        let one = dir.join("rt-one");
        rt::write_inputs(&one, &per[&i]);
        let alone = rt::check(&one, panic_sig).values().any(|(s, _)| *s == sig);
        let files = if alone { finding_files(&per[&i], &[]) } else { finding_files(&all, &[f.clone()]) };
        verdicts[i] = Verdict::Fail { sig, msg, errs: vec![], files };
    }
}

/// The `incr__` oracle (src/incr.rs) on the programs of a batch that passed the other oracles.
fn incr_batch(dir: &Path, srcs: &[String], obs: &[BTreeMap<String, Vec<String>>], verdicts: &mut [Verdict]) {
    let ok: Vec<usize> = (0..srcs.len()).filter(|&i| matches!(verdicts[i], Verdict::Pass { .. })).collect();
    if ok.is_empty() {
        return;
    }
    let per: BTreeMap<usize, Vec<Vec<(String, String)>>> =
        ok.iter().map(|&i| (i, incr::program_steps(i, &srcs[i], |p| check_file(p, &obs[i]).0))).collect();
    let steps_of = |progs: &[usize]| -> Vec<Vec<(String, String)>> {
        (0..=incr::EDITS)
            .map(|k| {
                let mut files = incr::common();
                for i in progs {
                    files.extend(per[i][k].iter().cloned());
                }
                files
            })
            .collect()
    };
    let finding_files = |steps: &[Vec<(String, String)>], only: &[String]| {
        let mut f: Vec<(String, String)> = Vec::new();
        for (k, files) in steps.iter().enumerate() {
            f.extend(files.iter().map(|(n, t)| (format!("incr/s{k}/{n}"), t.clone())));
        }
        f.push(("incr/only.txt".into(), only.iter().map(|o| format!("{o}\n")).collect()));
        f
    };
    let all = steps_of(&ok);
    let idir = dir.join("incr");
    incr::write_steps(&idir, &all);
    // The program alone reproduces it, or else the whole batch is the repro.
    let alone = |i: usize, sig: &str| -> Option<Vec<(String, String)>> {
        let steps = steps_of(&[i]);
        let w = dir.join("incr-one");
        incr::write_steps(&w, &steps);
        let hit = match incr::run(&w, &w, panic_sig) {
            Err((s, _)) => s == sig,
            Ok(checks) => incr::compare(&checks, None, panic_sig).values().any(|(s, _)| s == sig),
        };
        hit.then(|| finding_files(&steps, &[]))
    };
    match incr::run(&idir, &idir, panic_sig) {
        Err((sig, msg)) => {
            let i = ok[0];
            let files = alone(i, &sig).unwrap_or_else(|| finding_files(&all, &[]));
            verdicts[i] = Verdict::Fail { sig, msg, errs: vec![], files };
        }
        Ok(checks) => {
            let mut done = BTreeSet::new();
            for (f, (sig, msg)) in incr::compare(&checks, None, panic_sig) {
                let Some(i) = f.trim_end_matches(".py").split_once('_').and_then(|(_, n)| n.parse::<usize>().ok()) else { continue };
                if !per.contains_key(&i) || !done.insert(i) {
                    continue;
                }
                let only = vec![format!("c_{i}.py"), format!("u_{i}.py"), format!("m_{i}.py")];
                let files = alone(i, &sig).unwrap_or_else(|| finding_files(&all, &only));
                verdicts[i] = Verdict::Fail { sig, msg, errs: vec![], files };
            }
        }
    }
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
            Verdict::Fail { sig: s, msg, errs, .. } if s == sig && (!sig.starts_with("fp__") || errs.iter().all(|e| allowed.contains(e))) => {
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
/// the message or of one of the program's errors (and every `also` substring in that same string).
#[derive(serde::Deserialize)]
struct Known {
    name: String,
    sig: String,
    msg: String,
    #[serde(default)]
    also: Vec<String>,
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
    let hit = |k: &Known, s: &str| s.contains(&k.msg) && k.also.iter().all(|a| s.contains(a.as_str()));
    known.iter().find(|k| sig.starts_with(&k.sig) && (hit(k, msg) || errs.iter().any(|e| hit(k, e)))).map(|k| k.name.as_str())
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
                let mut todo: Vec<(String, String, u64, Tree, u32, Vec<String>, Vec<(String, String)>)> = Vec::new();
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
                            Verdict::Fail { sig, msg, errs, files } => {
                                if let Some(k) = known_name(&known, sig, msg, errs) {
                                    *st.fails.entry(format!("known__{k}")).or_default() += 1;
                                    continue;
                                }
                                *st.fails.entry(sig.clone()).or_default() += 1;
                                let mut sn = seen.lock().unwrap();
                                let c = sn.entry(sig.clone()).or_default();
                                if *c < 3 {
                                    *c += 1;
                                    todo.push((sig.clone(), msg.clone(), *s, t.clone(), *c, errs.clone(), files.clone()));
                                }
                            }
                        }
                    }
                }
                for (sig, msg, s, t, n, errs, files) in todo {
                    let fdir = out.join("findings").join(&sig).join(n.to_string());
                    fs::create_dir_all(&fdir).unwrap();
                    for (name, text) in &files {
                        let p = fdir.join(name);
                        fs::create_dir_all(p.parent().unwrap()).unwrap();
                        fs::write(p, text).unwrap();
                    }
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
                    "rt_unsafe_fix_skips": rt::UNSAFE_FIX_SKIPS.load(std::sync::atomic::Ordering::Relaxed),
                    "ren_requests": ren::REQUESTS.load(std::sync::atomic::Ordering::Relaxed),
                    "ren_variants": *ren::VARIANTS.lock().unwrap(),
                    "hint_variants": *hint::VARIANTS.lock().unwrap(),
                    "lsp": *lsp::STATS.lock().unwrap(),
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
    use_venv();
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
            if !fdir.join("incr").is_dir() {
                // SAFETY: single-threaded here.
                unsafe { std::env::set_var("TYZ_INCR", "0") };
            }
            if !fdir.join("rt").is_dir() {
                // SAFETY: single-threaded here.
                unsafe { std::env::set_var("TYZ_RT", "0") };
            }
            if !fdir.join("ren").is_dir() {
                // SAFETY: single-threaded here.
                unsafe { std::env::set_var("TYZ_REN", "0") };
            }
            if !fdir.join("lsp").is_dir() {
                // SAFETY: single-threaded here.
                unsafe { std::env::set_var("TYZ_LSP", "0") };
            }
            if !fdir.join("hint").is_dir() {
                // SAFETY: single-threaded here.
                unsafe { std::env::set_var("TYZ_HINT", "0") };
            } else if let Some(k) = sig.rsplit("__").next().filter(|k| hint::is_bad(k)) {
                // SAFETY: single-threaded here. The known-bad shape the finding shows, in every program.
                unsafe { std::env::set_var("TYZ_HINT_FORCE", k) };
            }
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
                std::env::set_var("TYZ_TYINC", build.join("bin/tyinc"));
                std::env::set_var("TYZ_INCR", "0");
                std::env::set_var("TYZ_RT", "0");
                std::env::set_var("TYZ_REN", "0");
                std::env::set_var("TYZ_HINT", "0");
                std::env::set_var("TYZ_LSP", "0");
            }
            if fdir.join("lsp").is_dir() {
                std::process::exit(lsp::repro(&fdir, &sig, panic_sig));
            }
            use_venv();
            if fdir.join("incr").is_dir() {
                std::process::exit(incr::repro(&fdir, &sig, panic_sig));
            }
            if fdir.join("rt").is_dir() {
                std::process::exit(rt::repro(&fdir, &sig, panic_sig));
            }
            if fdir.join("ren").is_dir() {
                std::process::exit(ren::repro(&fdir, &sig, panic_sig));
            }
            if fdir.join("hint").is_dir() {
                std::process::exit(hint::repro(&fdir, &sig, panic_sig));
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
        Some("rt-inputs") => {
            // tyz rt-inputs <file.py>: the rt oracle's inputs for a program
            let src = fs::read_to_string(&args[2]).unwrap();
            for (n, t) in rt::inputs(0, &src) {
                println!("# ---- {n}\n{t}");
            }
        }
        Some("ren-reqs") => {
            // tyz ren-reqs <file.py> [k]: the ren oracle's rename requests for a program
            let src = fs::read_to_string(&args[2]).unwrap();
            let k = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
            for r in ren::requests("p_0.py", &src, k) {
                println!("{} @{} (line {})", r.old, r.offset, src[..r.offset].lines().count().max(1));
            }
        }
        Some("incr-edit") => {
            // tyz incr-edit <file.py>: the incr oracle's steps for a program (without probes' asserts)
            let src = fs::read_to_string(&args[2]).unwrap();
            for (k, files) in incr::program_steps(0, &src, |p| p.to_string()).iter().enumerate() {
                for (n, t) in files {
                    println!("# ---- step {k}: {n}\n{t}");
                }
            }
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
