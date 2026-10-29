// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The `ren__` oracle: ty's rename (`ty_ide::rename`, what the language server does on F2) must
//! give an equivalent program. For programs that passed the other oracles, a few of their own names
//! (classes, functions, methods, attributes, fields, parameters, locals; picked at one of their
//! occurrences, as a user would) are renamed (to a fresh name as long as the old one) through `bin/tyinc --rename`, and:
//!
//! - every edit replaces the old name in the file the request came from (`ren__edit__<kind>`,
//!   `ren__crossfile__<kind>`: the programs of a batch share one project but never import each
//!   other), and one edit covers the occurrence the rename started from (`ren__self__<kind>`);
//! - CPython runs the renamed program cleanly with the same probe values, the old name put back
//!   (`ren__runtime__<kind>`);
//! - ty reports no error on it (`ren__check-<rule>__<kind>`).
//!
//! Names that occur in a string literal (`hasattr`, TypedDict keys, functional enums) and names
//! defined in more than one place (overrides, protocol members and their implementations, which a
//! rename of one declaration may legitimately leave alone) are not picked. `<kind>` is where the
//! name is declared: `class`, `func`, `method`, `attr`, `param`, `var`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::incr;


/// Requests sent, and renamed variants checked by kind (for stats.json).
pub static REQUESTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static VARIANTS: std::sync::Mutex<BTreeMap<&'static str, usize>> = std::sync::Mutex::new(BTreeMap::new());

/// Whether the oracle runs: TYZ_REN=0 turns it off, and it needs bin/tyinc.
pub fn enabled() -> bool {
    std::env::var("TYZ_REN").map(|v| v != "0").unwrap_or(true) && Path::new(&incr::tyinc_bin()).is_file()
}

#[derive(Clone, Debug)]
pub struct Req {
    pub file: String,
    pub offset: usize,
    pub old: String,
    /// The new name: as long as the old one (programs compute `len(C.__name__)`), found nowhere
    /// in the program's text.
    pub new: String,
}

/// A name as long as `old` that occurs nowhere in `ctx`.
fn fresh(old: &str, ctx: &str) -> Option<String> {
    let mut c: Vec<char> = old.chars().collect();
    for i in (0..c.len()).rev() {
        for x in ['Q', 'X', 'Z', 'q', 'x', 'z', 'J', 'j'] {
            if c[i] == x {
                continue;
            }
            let keep = c[i];
            c[i] = x;
            let n: String = c.iter().collect();
            if !ctx.contains(&n) {
                return Some(n);
            }
            c[i] = keep;
        }
    }
    None
}

const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else",
    "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
    "return", "try", "while", "with", "yield", "match", "case", "type", "self", "cls", "main",
    // pydantic reads it by name (an override of `BaseModel.model_config`).
    "model_config",
];

/// Identifier occurrences outside strings and comments, and the words inside string literals.
pub fn lex(src: &str) -> (Vec<(usize, String)>, BTreeSet<String>) {
    let b = src.as_bytes();
    let mut ids = Vec::new();
    let mut words = BTreeSet::new();
    let word = |s: &str, words: &mut BTreeSet<String>| {
        for w in s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            if !w.is_empty() {
                words.insert(w.to_string());
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'#' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'"' || c == b'\'' {
            let triple = b[i..].starts_with(&[c, c, c]);
            let q = if triple { 3 } else { 1 };
            let start = i + q;
            let mut j = start;
            loop {
                if j >= b.len() {
                    break;
                }
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if (triple && b[j..].starts_with(&[c, c, c])) || (!triple && (b[j] == c || b[j] == b'\n')) {
                    break;
                }
                j += 1;
            }
            word(&src[start..j.min(b.len())], &mut words);
            i = (j + q).min(b.len());
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            // A string prefix (f"", b'', r"") is not a name.
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') && i - s <= 2 && src[s..i].chars().all(|c| "rbfuRBFU".contains(c)) {
                continue;
            }
            if s == 0 || !b[s - 1].is_ascii_digit() {
                ids.push((s, src[s..i].to_string()));
            }
        } else {
            i += 1;
        }
    }
    (ids, words)
}

/// Lines that declare `name` (`def name(`, `class name`, `name: T` / `self.name` assignment in a class
/// or method).
fn declarations(src: &str, name: &str) -> usize {
    src.lines()
        .filter(|l| {
            let t = l.trim_start();
            let t = t.strip_prefix("async ").unwrap_or(t);
            let after = |p: &str| t.strip_prefix(p).and_then(|r| r.strip_prefix(name)).map(|r| r.trim_start().chars().next());
            matches!(after("def "), Some(Some('(' | '[')))
                || matches!(after("class "), Some(Some('(' | '[' | ':')))
                || (l.starts_with(' ') && matches!(after(""), Some(Some(':'))))
                || matches!(after("self."), Some(Some(':' | '=')))
        })
        .count()
}

/// Up to `k` rename requests for the program `file`: distinct names, each at a random occurrence.
pub fn requests(file: &str, src: &str, k: usize) -> Vec<Req> {
    requests_in(file, src, src, src, k)
}

/// `requests`, with string words and declarations counted over `ctx` (all of a split program).
/// String words come from `wctx` (the unquoted program, when annotations were quoted).
fn requests_in(file: &str, src: &str, ctx: &str, wctx: &str, k: usize) -> Vec<Req> {
    let (ids, _) = lex(src);
    let (_, words) = lex(wctx);
    let mut by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (o, n) in &ids {
        by_name.entry(n.as_str()).or_default().push(*o);
    }
    // Names the program binds itself (not imported): followed by `:`/`=`/`:=`, or after
    // `def`/`class`/`as`/`for`/`*`.
    let b = src.as_bytes();
    let mut bound = BTreeSet::new();
    let mut imported = BTreeSet::new();
    for (o, n) in &ids {
        let ls = src[..*o].rfind('\n').map_or(0, |p| p + 1);
        let line = src[ls..].lines().next().unwrap_or("").trim_start();
        if line.starts_with("import ") || line.starts_with("from ") {
            // Names imported from the program's own modules (the split ones) are its own.
            let own = ["m_", "md_", "mp_", "."].iter().any(|p| line.strip_prefix("from ").is_some_and(|r| r.starts_with(p)));
            if !own {
                imported.insert(n.as_str());
            } else if !["from", "import", "as"].contains(&n.as_str()) && !n.starts_with("m_") && !n.starts_with("md_") && !n.starts_with("mp_") {
                bound.insert(n.as_str());
            }
            continue;
        }
        let mut e = o + n.len();
        while e < b.len() && b[e] == b' ' {
            e += 1;
        }
        let after = b.get(e).copied();
        let after2 = b.get(e + 1).copied();
        let before = src[ls..*o].trim_end();
        if (after == Some(b'=') && after2 != Some(b'=')) || after == Some(b':')
            || ["def", "class", "as", "for"].iter().any(|k| before.ends_with(k) && !before[..before.len() - k.len()].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
            || before.ends_with('*')
            || before.ends_with(", ") && before.contains("for ")
        {
            bound.insert(n.as_str());
        }
    }
    let names: Vec<&str> = by_name
        .keys()
        .copied()
        // Module names of a split program: ty renames `from . import <submodule>` bindings
        // (rename.rs: "TODO(submodule-imports): we should refuse to rename this").
        .filter(|n| !["m_", "md_", "mp_", "mpd_"].iter().any(|p| n.starts_with(p)))
        .filter(|n| bound.contains(n) && !imported.contains(n) && !n.starts_with('_') && !KEYWORDS.contains(n) && !words.contains(*n) && declarations(ctx, n) <= 1)
        .collect();
    let mut h = incr::hash(src);
    let mut next = || {
        h = h.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (h >> 33) as usize
    };
    let mut out = Vec::new();
    let mut used = BTreeSet::new();
    for _ in 0..k * 4 {
        if out.len() >= k || names.is_empty() {
            break;
        }
        let n = names[next() % names.len()];
        if !used.insert(n) {
            continue;
        }
        let occ = &by_name[n];
        let Some(new) = fresh(n, ctx) else { continue };
        out.push(Req { file: file.to_string(), offset: occ[next() % occ.len()], old: n.to_string(), new });
    }
    out
}

/// Where the renamed symbol is declared, from the text around the edits.
fn kind(src: &str, edits: &[(usize, usize)]) -> &'static str {
    let line_of = |o: usize| {
        let s = src[..o].rfind('\n').map_or(0, |p| p + 1);
        let e = src[o..].find('\n').map_or(src.len(), |p| o + p);
        (&src[s..o], &src[o..e])
    };
    let mut k = "var";
    for &(s, e) in edits {
        let (before, after) = line_of(s);
        let after = &after[e - s..];
        let t = before.trim_start();
        if t == "def " || t == "async def " {
            return if before.starts_with(' ') { "method" } else { "func" };
        }
        if t == "class " {
            return "class";
        }
        if (t.starts_with("def ") || t.starts_with("async def ")) && before.contains('(') {
            k = "param";
        } else if before.ends_with("self.") && k == "var" {
            k = "attr";
        } else if t.is_empty() && before.starts_with(' ') && after.trim_start().starts_with(':') && k == "var" {
            k = "attr";
        }
    }
    k
}

/// Runs `tyinc --rename` on the files of `indir` for `reqs`: one answer per request, or
/// Err((signature, message)) when tyinc dies.
pub fn run_tyinc(work: &Path, indir: &Path, reqs: &[Req], panic_sig: impl Fn(&str) -> String) -> Result<Vec<Value>, (String, String)> {
    fs::create_dir_all(work).unwrap();
    let rj = work.join("req.json");
    let v: Vec<Value> = reqs.iter().map(|r| json!({"file": r.file, "offset": r.offset, "new": r.new})).collect();
    fs::write(&rj, serde_json::to_string(&v).unwrap()).unwrap();
    let t0 = Instant::now();
    let mut child = Command::new(incr::tyinc_bin())
        .arg("--rename")
        .arg(work.join("proj"))
        .arg(indir)
        .arg(&rj)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("tyinc");
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let ho = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut so, &mut s);
        s
    });
    let he = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut se, &mut s);
        s
    });
    let timeout = Duration::from_secs(120);
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
    let stdout = ho.join().unwrap_or_default();
    let stderr = he.join().unwrap_or_default();
    let answers: Vec<Value> = stdout.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    match status {
        None => Err(("crash__hang__tyinc".into(), format!("tyinc --rename ran past {timeout:?}"))),
        Some(s) if !s.success() || answers.len() != reqs.len() => {
            let p = stderr.lines().find(|l| l.starts_with("Panicked at ")).map(str::to_string);
            match p {
                Some(p) => Err((panic_sig(&p), p)),
                None => Err((format!("crash__exit__tyinc_{}", s.code().map(|c| c.to_string()).unwrap_or("signal".into())), stderr.chars().take(2000).collect())),
            }
        }
        Some(_) => Ok(answers),
    }
}

/// A renamed variant: the request's index, its file name (flat batches), the renamed files, its kind.
pub struct Variant {
    pub req: usize,
    pub name: String,
    pub files: BTreeMap<String, String>,
    pub kind: &'static str,
}

/// Applies the answers: the variants to run and check, and the findings the edits alone show
/// (request index -> (signature, message)). With `multi`, the files of `srcs` are one program
/// (modules importing each other), so edits may go to any of them; otherwise each is its own.
pub fn apply(srcs: &BTreeMap<String, String>, multi: bool, reqs: &[Req], answers: &[Value], panic_sig: impl Fn(&str) -> String) -> (Vec<Variant>, BTreeMap<usize, (String, String)>) {
    let mut vs = Vec::new();
    let mut found = BTreeMap::new();
    for (j, (r, a)) in reqs.iter().zip(answers).enumerate() {
        if let Some(p) = a["panic"].as_str() {
            found.insert(j, (panic_sig(p), p.to_string()));
            continue;
        }
        let (Some(_), Some(edits)) = (a["range"].as_array(), a["edits"].as_array()) else { continue };
        let mut per: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
        let mut bad = None;
        for e in edits {
            let (f, s, t) = (e[0].as_str().unwrap_or(""), e[1].as_u64().unwrap_or(0) as usize, e[2].as_u64().unwrap_or(0) as usize);
            let ok_file = if multi { srcs.contains_key(f) } else { f == r.file };
            if !ok_file {
                bad.get_or_insert(("crossfile", format!("renaming `{}` at {}@{} edits {f}@{s}..{t}", r.old, r.file, r.offset)));
            } else if srcs[f].get(s..t) != Some(r.old.as_str()) {
                bad.get_or_insert(("edit", format!("renaming `{}` at {}@{} replaces {:?} at {f}@{s}..{t}", r.old, r.file, r.offset, srcs[f].get(s..t))));
            } else {
                per.entry(f.to_string()).or_default().push((s, t));
            }
        }
        let mut k = "var";
        for (f, es) in per.iter_mut() {
            es.sort();
            es.dedup();
            let k2 = kind(&srcs[f], es);
            if k == "var" || (k2 != "var" && f == &r.file) {
                k = k2;
            }
        }
        // `-q`: one file with its annotations quoted; `-x`: split over modules.
        let k = if !multi { k } else if srcs.keys().any(|f| f.starts_with("q_")) { leak(format!("{k}-q")) } else { leak(format!("{k}-x")) };
        if let Some((what, msg)) = bad {
            found.insert(j, (format!("ren__{what}__{k}"), msg));
            continue;
        }
        if per.is_empty() {
            continue;
        }
        if !per.get(&r.file).is_some_and(|es| es.iter().any(|&(s, t)| s <= r.offset && r.offset < t)) {
            found.insert(j, (format!("ren__self__{k}"), format!("renaming `{}` at {}@{} does not edit that occurrence ({} edits)", r.old, r.file, r.offset, edits.len())));
            continue;
        }
        let mut files = BTreeMap::new();
        for (f, es) in &per {
            let mut text = srcs[f].clone();
            for &(s, t) in es.iter().rev() {
                text.replace_range(s..t, &r.new);
            }
            files.insert(f.clone(), text);
        }
        vs.push(Variant { req: j, name: format!("r{j}_{}", r.file), files, kind: k });
    }
    (vs, found)
}

pub fn leak(s: String) -> &'static str {
    static KINDS: std::sync::Mutex<BTreeSet<&'static str>> = std::sync::Mutex::new(BTreeSet::new());
    let mut ks = KINDS.lock().unwrap();
    if let Some(k) = ks.get(s.as_str()) {
        return k;
    }
    let k: &'static str = Box::leak(s.into_boxed_str());
    ks.insert(k);
    k
}

/// Runs the variants under CPython and ty in `work`: request index -> (signature, message).
/// `obs`: the original programs' probe values, by file.
pub fn judge(work: &Path, reqs: &[Req], vs: &[Variant], obs: &BTreeMap<String, Value>) -> BTreeMap<usize, (String, String)> {
    let mut found = BTreeMap::new();
    if vs.is_empty() {
        return found;
    }
    let rdir = work.join("run");
    let cdir = work.join("check");
    let _ = fs::remove_dir_all(&rdir);
    let _ = fs::remove_dir_all(&cdir);
    fs::create_dir_all(&rdir).unwrap();
    fs::create_dir_all(&cdir).unwrap();
    fs::write(rdir.join("_rt.py"), crate::RT).unwrap();
    fs::write(rdir.join("runner.py"), crate::RUNNER).unwrap();
    fs::write(cdir.join("_rt.py"), incr::RT_STUB).unwrap();
    for v in vs {
        let text = v.files.values().next().unwrap();
        fs::write(rdir.join(&v.name), text).unwrap();
        fs::write(cdir.join(&v.name), text).unwrap();
    }
    let names: Vec<&str> = vs.iter().map(|v| v.name.as_str()).collect();
    let out = Command::new(crate::py_bin()).arg("-I").arg(rdir.join("runner.py")).arg(&rdir).args(&names).stderr(Stdio::null()).output().expect("python");
    let mut runs: BTreeMap<String, Value> = BTreeMap::new();
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        if let Ok(v) = serde_json::from_str::<Value>(l) {
            let f = Path::new(v["file"].as_str().unwrap_or("")).file_name().unwrap().to_string_lossy().to_string();
            runs.insert(f, v);
        }
    }
    let (stdout, fail) = crate::run_ty(&cdir, Duration::from_secs(60 + vs.len() as u64));
    let diags = crate::parse_diags(&stdout);
    for v in vs {
        let r = &reqs[v.req];
        let what = format!("renaming `{}` at {}@{}", r.old, r.file, r.offset);
        if let Some((sig, msg)) = &fail {
            found.insert(v.req, (sig.clone(), msg.clone()));
            continue;
        }
        if let Some(d) = diags.get(&v.name).and_then(|ds| ds.iter().find(|d| d.rule == "panic" || d.severity == "fatal")) {
            found.insert(v.req, (crate::panic_sig(&d.msg), d.msg.clone()));
            continue;
        }
        let Some(run) = runs.get(&v.name) else { continue };
        if run["status"] == "timeout" || run["status"] == "crash" {
            continue;
        }
        if run["status"] != "ok" {
            let e = run["error"].as_str().unwrap_or("?").replace(&r.new, &r.old);
            found.insert(v.req, (format!("ren__runtime__{}", v.kind), format!("{what}: the renamed program raises {e}")));
            continue;
        }
        let got = norm(&run["obs"], r);
        let want = norm(&obs[&r.file], r);
        if got != want {
            found.insert(v.req, (format!("ren__runtime__{}", v.kind), format!("{what}: probe values differ: {}", first_diff(&got, &want))));
            continue;
        }
        // A renamed parameter of an override or an overload implementation alone makes the
        // signatures incompatible: what the user asked for, not a broken rename.
        let expected = |d: &crate::Diag| v.kind == "param" && matches!(d.rule.as_str(), "invalid-method-override" | "invalid-overload");
        if let Some(d) = diags.get(&v.name).and_then(|ds| ds.iter().find(|d| d.severity == "error" && !expected(d))) {
            found.insert(v.req, (format!("ren__check-{}__{}", d.rule, v.kind), format!("{what}: line {}: {}", d.line, d.msg.replace(&r.new, &r.old))));
        }
    }
    found
}

/// Probe values with the new name put back and the contents of string literals blanked: a renamed class,
/// enum member or field changes the values of `__name__`, `.name` and reprs (and `.upper()` of
/// them), which is not a change in behaviour.
fn norm(obs: &Value, r: &Req) -> String {
    let s = serde_json::to_string(obs).unwrap().replace(&r.new, &r.old);
    let v: Value = serde_json::from_str(&s).unwrap();
    let blank = |t: &str| {
        let mut out = String::new();
        let mut it = t.chars().peekable();
        while let Some(c) = it.next() {
            out.push(c);
            if c == '\'' || c == '"' {
                while let Some(d) = it.next() {
                    if d == '\\' {
                        it.next();
                    } else if d == c {
                        out.push('S');
                        out.push(c);
                        break;
                    }
                }
            }
        }
        out
    };
    let m: BTreeMap<String, Vec<String>> = v
        .as_object()
        .map(|o| o.iter().map(|(k, xs)| {
                let mut b: Vec<String> = xs.as_array().into_iter().flatten().map(|x| blank(x.as_str().unwrap_or(""))).collect();
                b.sort();
                b.dedup();
                (k.clone(), b)
            })
            .collect())
        .unwrap_or_default();
    serde_json::to_string(&m).unwrap()
}

/// The first probe whose (normalized) values differ: `probe k: <renamed> vs <original>`.
fn first_diff(got: &str, want: &str) -> String {
    let g: BTreeMap<String, Vec<String>> = serde_json::from_str(got).unwrap_or_default();
    let w: BTreeMap<String, Vec<String>> = serde_json::from_str(want).unwrap_or_default();
    let keys: BTreeSet<&String> = g.keys().chain(w.keys()).collect();
    for k in keys {
        if g.get(k) != w.get(k) {
            return format!("probe {k}: {:?} vs {:?}", g.get(k), w.get(k));
        }
    }
    String::new()
}

/// Rename from another occurrence the first rename edited: the same symbol, so the same edits
/// (`ren__asym__<kind>`). Request index -> finding.
fn asym(work: &Path, indir: &Path, reqs: &[Req], answers: &[Value], vs: &[Variant], panic_sig: impl Fn(&str) -> String) -> Result<BTreeMap<usize, (String, String)>, (String, String)> {
    let set = |a: &Value| -> BTreeSet<(String, u64, u64)> {
        a["edits"].as_array().into_iter().flatten().map(|e| (e[0].as_str().unwrap_or("").to_string(), e[1].as_u64().unwrap_or(0), e[2].as_u64().unwrap_or(0))).collect()
    };
    let mut follow: Vec<(usize, Req)> = Vec::new();
    for v in vs {
        let r = &reqs[v.req];
        let es: Vec<(String, u64, u64)> = set(&answers[v.req]).into_iter().filter(|(f, s, t)| !(f == &r.file && (*s as usize) <= r.offset && r.offset < *t as usize)).collect();
        if es.is_empty() {
            continue;
        }
        let (f, s, _) = &es[(incr::hash(&format!("{}{}", r.file, r.offset)) % es.len() as u64) as usize];
        follow.push((v.req, Req { file: f.clone(), offset: *s as usize, old: r.old.clone(), new: r.new.clone() }));
    }
    let mut found = BTreeMap::new();
    if follow.is_empty() {
        return Ok(found);
    }
    let rs: Vec<Req> = follow.iter().map(|(_, r)| r.clone()).collect();
    let ans = run_tyinc(&work.join("asym"), indir, &rs, &panic_sig)?;
    for ((j, r2), a2) in follow.iter().zip(&ans) {
        if let Some(p) = a2["panic"].as_str() {
            found.insert(*j, (panic_sig(p), p.to_string()));
            continue;
        }
        let (one, two) = (set(&answers[*j]), set(a2));
        if one != two {
            let r = &reqs[*j];
            let k = vs.iter().find(|v| v.req == *j).map_or("var", |v| v.kind);
            let only1: Vec<_> = one.difference(&two).take(3).collect();
            let only2: Vec<_> = two.difference(&one).take(3).collect();
            found.insert(*j, (format!("ren__asym__{k}"), format!(
                "renaming `{}` at {}@{} edits {} places, at {}@{} {} places (only the first: {only1:?}; only the second: {only2:?})",
                r.old, r.file, r.offset, one.len(), r2.file, r2.offset, two.len()
            )));
        }
    }
    Ok(found)
}

/// The project files for tyinc: the programs plus the probe stub.
pub fn write_inputs(dir: &Path, files: &BTreeMap<String, String>) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("_rt.py"), incr::RT_STUB).unwrap();
    fs::write(dir.join("ty.toml"), incr::TY_TOML).unwrap();
    for (n, t) in files {
        let p = dir.join(n);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, t).unwrap();
    }
}

/// The oracle on one program split over modules (`unit`: path -> text, `entry` imports it):
/// the renamed program must run like the unrenamed split (not like the single file: the probes
/// render `__main__`'s classes), and check without error. Request -> finding.
pub fn check_unit(work: &Path, unit: &BTreeMap<String, String>, entry: &str, reqs: &[Req], panic_sig: impl Fn(&str) -> String + Copy) -> Result<BTreeMap<usize, (String, String)>, (String, String)> {
    let mut found = BTreeMap::new();
    let run = |dir: &Path, files: &BTreeMap<String, String>| -> Option<Value> {
        let _ = fs::remove_dir_all(dir);
        for (n, t) in files {
            let p = dir.join(n);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, t).unwrap();
        }
        fs::write(dir.join("_rt.py"), crate::RT).unwrap();
        fs::write(dir.join("runner.py"), crate::RUNNER).unwrap();
        let out = Command::new(crate::py_bin()).arg("-I").arg(dir.join("runner.py")).arg(dir).arg(entry).stderr(Stdio::null()).output().ok()?;
        String::from_utf8_lossy(&out.stdout).lines().find_map(|l| serde_json::from_str::<Value>(l).ok())
    };
    let base = run(&work.join("base"), unit);
    let Some(base) = base.filter(|b| b["status"] == "ok") else { return Ok(found) };
    let indir = work.join("in");
    write_inputs(&indir, unit);
    let answers = run_tyinc(work, &indir, reqs, panic_sig)?;
    let (vs, f0) = apply(unit, true, reqs, &answers, panic_sig);
    found.extend(f0);
    for (j, f) in asym(work, &indir, reqs, &answers, &vs, panic_sig)? {
        found.entry(j).or_insert(f);
    }
    REQUESTS.fetch_add(reqs.len(), std::sync::atomic::Ordering::Relaxed);
    for v in &vs {
        *VARIANTS.lock().unwrap().entry(v.kind).or_default() += 1;
        let r = &reqs[v.req];
        let what = format!("renaming `{}` at {}@{}", r.old, r.file, r.offset);
        let mut files = unit.clone();
        files.extend(v.files.clone());
        let Some(got) = run(&work.join("run"), &files) else { continue };
        if got["status"] == "timeout" || got["status"] == "crash" {
            continue;
        }
        if got["status"] != "ok" {
            let e = got["error"].as_str().unwrap_or("?").replace(&r.new, &r.old);
            found.insert(v.req, (format!("ren__runtime__{}", v.kind), format!("{what}: the renamed program raises {e}")));
            continue;
        }
        if norm(&got["obs"], r) != norm(&base["obs"], r) {
            found.insert(v.req, (format!("ren__runtime__{}", v.kind), format!("{what}: probe values differ: {}", first_diff(&norm(&got["obs"], r), &norm(&base["obs"], r)))));
            continue;
        }
        let cdir = work.join("check");
        write_inputs(&cdir, &files);
        let (stdout, fail) = crate::run_ty(&cdir, Duration::from_secs(60));
        if let Some(f) = fail {
            found.insert(v.req, f);
            continue;
        }
        let diags = crate::parse_diags(&stdout);
        let expected = |d: &crate::Diag| v.kind.starts_with("param") && matches!(d.rule.as_str(), "invalid-method-override" | "invalid-overload");
        if let Some((f, d)) = diags.iter().flat_map(|(f, ds)| ds.iter().map(move |d| (f, d))).find(|(_, d)| d.rule == "panic" || d.severity == "fatal") {
            let _ = f;
            found.insert(v.req, (crate::panic_sig(&d.msg), d.msg.clone()));
        } else if let Some((f, d)) = diags.iter().flat_map(|(f, ds)| ds.iter().map(move |d| (f, d))).find(|(_, d)| d.severity == "error" && !expected(d)) {
            found.insert(v.req, (format!("ren__check-{}__{}", d.rule, v.kind), format!("{what}: {f}:{}: {}", d.line, d.msg.replace(&r.new, &r.old))));
        }
    }
    Ok(found)
}

/// The whole oracle on `srcs` (file -> program) with their probe values: request -> finding.
pub fn check(work: &Path, srcs: &BTreeMap<String, String>, reqs: &[Req], obs: &BTreeMap<String, Value>, panic_sig: impl Fn(&str) -> String + Copy) -> Result<BTreeMap<usize, (String, String)>, (String, String)> {
    let indir = work.join("in");
    write_inputs(&indir, srcs);
    let answers = run_tyinc(work, &indir, reqs, panic_sig)?;
    let (vs, mut found) = apply(srcs, false, reqs, &answers, panic_sig);
    REQUESTS.fetch_add(reqs.len(), std::sync::atomic::Ordering::Relaxed);
    for v in &vs {
        *VARIANTS.lock().unwrap().entry(v.kind).or_default() += 1;
    }
    for (j, f) in judge(work, reqs, &vs, obs) {
        found.entry(j).or_insert(f);
    }
    for (j, f) in asym(work, &indir, reqs, &answers, &vs, panic_sig)? {
        found.entry(j).or_insert(f);
    }
    Ok(found)
}

/// Finding files: `ren/in/*` (the programs), `ren/req.json` (the one request), `ren/obs.json`.
pub fn finding_files(srcs: &BTreeMap<String, String>, req: &Req, obs: &BTreeMap<String, Value>) -> Vec<(String, String)> {
    let mut f: Vec<(String, String)> = srcs.iter().map(|(n, t)| (format!("ren/in/{n}"), t.clone())).collect();
    f.push(("ren/req.json".into(), json!({"file": req.file, "offset": req.offset, "old": req.old, "new": req.new}).to_string()));
    f.push(("ren/obs.json".into(), serde_json::to_string(obs).unwrap()));
    f
}

/// `repro` for a ren finding: 1 when the rename still shows `sig`, 0 when not, 2 when undecided.
pub fn repro(fdir: &Path, sig: &str, panic_sig: impl Fn(&str) -> String + Copy) -> i32 {
    if !Path::new(&incr::tyinc_bin()).is_file() {
        return 2;
    }
    let d = fdir.join("ren");
    let r: Value = serde_json::from_str(&fs::read_to_string(d.join("req.json")).unwrap()).unwrap();
    let req = Req { file: r["file"].as_str().unwrap().into(), offset: r["offset"].as_u64().unwrap() as usize, old: r["old"].as_str().unwrap().into(), new: r["new"].as_str().unwrap().into() };
    let work: PathBuf = std::env::temp_dir().join(format!("tyz-ren-repro-{}", std::process::id()));
    if d.join("unit").is_dir() {
        let mut unit = BTreeMap::new();
        read_tree(&d.join("unit"), "", &mut unit);
        let entry = fs::read_to_string(d.join("entry.txt")).unwrap_or_default().trim().to_string();
        let res = check_unit(&work, &unit, &entry, &[req], panic_sig);
        let _ = fs::remove_dir_all(&work);
        return match res {
            Err((s, m)) => {
                println!("{s}: {m}");
                (s == sig) as i32
            }
            Ok(f) => {
                println!("{f:?}");
                f.values().any(|(s, _)| s == sig) as i32
            }
        };
    }
    let obs: BTreeMap<String, Value> = serde_json::from_str(&fs::read_to_string(d.join("obs.json")).unwrap()).unwrap();
    let mut srcs = BTreeMap::new();
    for e in fs::read_dir(d.join("in")).unwrap().flatten() {
        srcs.insert(e.file_name().to_string_lossy().to_string(), fs::read_to_string(e.path()).unwrap());
    }
    let res = check(&work, &srcs, &[req], &obs, panic_sig);
    let _ = fs::remove_dir_all(&work);
    match res {
        Err((s, m)) => {
            println!("{s}: {m}");
            (s == sig) as i32
        }
        Ok(f) => {
            println!("{f:?}");
            f.values().any(|(s, _)| s == sig) as i32
        }
    }
}

fn read_tree(dir: &Path, prefix: &str, out: &mut BTreeMap<String, String>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let n = format!("{prefix}{}", e.file_name().to_string_lossy());
        if e.path().is_dir() {
            read_tree(&e.path(), &format!("{n}/"), out);
        } else {
            out.insert(n, fs::read_to_string(e.path()).unwrap_or_default());
        }
    }
}

/// Finding files for a split program: `ren/unit/*`, `ren/entry.txt`, `ren/req.json`.
pub fn unit_files(unit: &BTreeMap<String, String>, entry: &str, req: &Req) -> Vec<(String, String)> {
    let mut f: Vec<(String, String)> = unit.iter().map(|(n, t)| (format!("ren/unit/{n}"), t.clone())).collect();
    f.push(("ren/entry.txt".into(), format!("{entry}\n")));
    f.push(("ren/req.json".into(), json!({"file": req.file, "offset": req.offset, "old": req.old, "new": req.new}).to_string()));
    f
}

/// Up to `k` requests on a split program, spread over its modules.
pub fn unit_requests(unit: &BTreeMap<String, String>, entry: &str, k: usize, wctx: Option<&str>) -> Vec<Req> {
    let files: Vec<&String> = unit.keys().filter(|f| f.as_str() != entry).collect();
    let ctx: String = unit.values().map(|t| format!("{t}\n")).collect();
    let mut out: Vec<Req> = Vec::new();
    for (n, f) in files.iter().enumerate() {
        let want = k / files.len() + usize::from(n < k % files.len());
        for r in requests_in(f, &unit[*f], &ctx, wctx.unwrap_or(&ctx), want + 2) {
            if out.len() < k && !out.iter().any(|o| o.old == r.old) && out.iter().filter(|o| &o.file == *f).count() < want.max(1) {
                out.push(r);
            }
        }
    }
    out
}
