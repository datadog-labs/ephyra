// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The `hint__` oracle: the edits ty's inlay hints offer (`ty_ide::inlay_hints`, what an editor
//! inserts when the user double-clicks a hint) must give an equivalent program. For programs that
//! passed the other oracles, `bin/tyinc --hints` lists the hints of each, and their edits are
//! applied, a variant per kind:
//!
//! - `argname`: every call's `name=` edit (ty offers it on a call's last positional argument that
//!   is not starred, turning it into a keyword argument);
//! - `vartype`: the `: T` annotation (and the imports it needs) on each assignment target bound
//!   only once in the program (a second binding of another type would fail the inserted
//!   declaration: the user asked for that); `vartype-class` for targets in a class body, where an
//!   annotation turns a class attribute into a dataclass/NamedTuple field.
//!
//! CPython must run the variant cleanly with the same probe values (`hint__runtime__<kind>`) and ty
//! must report no error on it (`hint__check-<rule>__<kind>`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::incr;

/// Variants checked, by kind (for stats.json).
pub static VARIANTS: std::sync::Mutex<BTreeMap<&'static str, usize>> = std::sync::Mutex::new(BTreeMap::new());

/// Whether the oracle runs: TYZ_HINT=0 turns it off, and it needs bin/tyinc.
pub fn enabled() -> bool {
    std::env::var("TYZ_HINT").map(|v| v != "0").unwrap_or(true) && Path::new(&incr::tyinc_bin()).is_file()
}

/// `tyinc --hints` on the `p_*.py` files of `indir`: file -> its hints (a JSON array).
fn run_tyinc(work: &Path, indir: &Path, panic_sig: impl Fn(&str) -> String) -> Result<BTreeMap<String, Value>, (String, String)> {
    fs::create_dir_all(work).unwrap();
    let t0 = Instant::now();
    let mut child = Command::new(incr::tyinc_bin())
        .arg("--hints")
        .arg(work.join("proj"))
        .arg(indir)
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
    let mut out = BTreeMap::new();
    for l in stdout.lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        if let Some(p) = v["panic"].as_str() {
            return Err((panic_sig(p), p.to_string()));
        }
        out.insert(v["file"].as_str().unwrap_or("").to_string(), v["hints"].clone());
    }
    match status {
        None => Err(("crash__hang__tyinc".into(), format!("tyinc --hints ran past {timeout:?}"))),
        Some(s) if !s.success() => {
            let p = stderr.lines().find(|l| l.starts_with("Panicked at ")).map(str::to_string);
            match p {
                Some(p) => Err((panic_sig(&p), p)),
                None => Err((format!("crash__exit__tyinc_{}", s.code().map(|c| c.to_string()).unwrap_or("signal".into())), stderr.chars().take(2000).collect())),
            }
        }
        Some(_) => Ok(out),
    }
}

/// A variant: the program it comes from, its kind, its text, the hints applied (for the message).
pub struct Variant {
    pub file: String,
    pub kind: &'static str,
    pub text: String,
    pub applied: Vec<String>,
}

/// How many places bind `name` (assignment and annotation targets, augmented assignments,
/// parameters, `for`/`as`/`global`/`nonlocal`/`del`/`def`/`class`/`import` names). Over-counts
/// (keyword arguments, dict keys, slices): those targets are only left out.
/// With `attr`, the target is an attribute (`self.name = ...`): only `.name` targets and names
/// declared at the start of a line (class bodies) count.
fn bindings(src: &str, ids: &[(usize, String)], name: &str, attr: bool) -> usize {
    let mut n = 0;
    for (k, (s, w)) in ids.iter().enumerate() {
        if w != name {
            continue;
        }
        if attr && !(*s > 0 && src.as_bytes()[s - 1] == b'.') && !src[..*s].rsplit('\n').next().unwrap_or("").trim().is_empty() {
            continue;
        }
        let rest = src[s + w.len()..].trim_start_matches([' ', '\t']);
        let rb = rest.as_bytes();
        let after = (rb.first() == Some(&b'=') && rb.get(1) != Some(&b'='))
            || rb.first() == Some(&b':')
            || (rb.len() >= 2 && b"+-*/%&|^@<>".contains(&rb[0]) && (rb[1] == b'=' || (rb[1] == rb[0] && rb.get(2) == Some(&b'='))));
        let before = k > 0 && {
            let (ps, pw) = &ids[k - 1];
            matches!(pw.as_str(), "for" | "as" | "global" | "nonlocal" | "del" | "def" | "class" | "import")
                && src[ps + pw.len()..*s].bytes().all(|c| c == b' ' || c == b'\t' || c == b',' || c == b'(')
        };
        // `for a, b in`: every name of a `for` target list.
        let in_for = {
            let pre = &src[src[..*s].rfind('\n').map_or(0, |p| p + 1)..*s];
            pre.rfind("for ").is_some_and(|p| (p == 0 || !pre.as_bytes()[p - 1].is_ascii_alphanumeric()) && pre[p + 4..].bytes().all(|c| c.is_ascii_alphanumeric() || b"_, ()[]".contains(&c)))
        };
        if after || before || in_for {
            n += 1;
        }
    }
    n
}

/// Whether offset `pos` is in a class body: the closest less indented line above it opens a class.
fn in_class_body(src: &str, pos: usize) -> bool {
    let ls = src[..pos].rfind('\n').map_or(0, |p| p + 1);
    let line = &src[ls..];
    let ind = line.len() - line.trim_start_matches([' ', '\t']).len();
    if ind == 0 {
        return false;
    }
    for l in src[..ls].lines().rev() {
        let t = l.trim_start_matches([' ', '\t']);
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if l.len() - t.len() < ind {
            return t.starts_with("class ");
        }
    }
    false
}

/// Applies `edits` (`[s, e, text]`, offsets into `src`; repeated imports once).
fn apply_edits(src: &str, edits: &[(usize, usize, String)]) -> Option<String> {
    let mut seen = BTreeSet::new();
    let mut es: Vec<(usize, usize, usize, &str)> = Vec::new();
    for (k, (s, e, t)) in edits.iter().enumerate() {
        if seen.insert((*s, *e, t.clone())) {
            es.push((*s, *e, k, t));
        }
    }
    // Later edits at the same offset first, so the earlier ones end up in front of them.
    es.sort_by(|a, b| b.0.cmp(&a.0).then(b.2.cmp(&a.2)));
    let mut out = src.to_string();
    let mut limit = usize::MAX;
    for (s, e, _, t) in es {
        if e > out.len() || s > e || e > limit || !out.is_char_boundary(s) || !out.is_char_boundary(e) {
            return None;
        }
        out.replace_range(s..e, t);
        limit = s;
    }
    Some(out)
}

/// The shape of the argument a `name=` edit at `pos` goes to: `ok`, or one of the shapes ty's edit
/// is known to break (and that only 1 in TYZ_HINT_BAD_EVERY programs gets, in a variant of their
/// own): `parens` (a parenthesized argument: the edit goes inside the parentheses), `varargs`
/// (positional arguments follow it: the call fills a `*args` parameter), `typealiastype` (ty
/// rejects `TypeAliasType("A", value=...)`).
fn arg_shape(src: &str, pos: usize) -> &'static str {
    let b = src.as_bytes();
    let back = |mut i: usize| {
        while i > 0 && b[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        i
    };
    let i = back(pos);
    if i > 0 && b[i - 1] == b'(' {
        let j = back(i - 1);
        if j > 0 && (b[j - 1] == b',' || b[j - 1] == b'(') {
            return "parens";
        }
    }
    // The callee: the identifier before the unbalanced `(` left of `pos`.
    let mut depth = 0i32;
    let mut k = pos;
    while k > 0 {
        k -= 1;
        match b[k] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' if depth > 0 => depth -= 1,
            b'(' => break,
            _ => {}
        }
    }
    let e = back(k);
    let callee = &src[src[..e].rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map_or(0, |p| p + 1)..e];
    if callee == "TypeAliasType" {
        return "typealiastype";
    }
    // The argument's end: `,` or the call's `)` at depth 0 (strings skipped).
    let (mut depth, mut k) = (0i32, pos);
    while k < b.len() {
        match b[k] {
            q @ (b'"' | b'\'') => {
                k += 1;
                while k < b.len() && b[k] != q && b[k] != b'\n' {
                    k += if b[k] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => return "ok",
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                let rest = src[k + 1..].trim_start();
                let word = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
                let after = rest[word..].trim_start();
                let keyword = word > 0 && after.starts_with('=') && !after.starts_with("==");
                return if keyword || rest.starts_with("**") || rest.starts_with(')') { "ok" } else { "varargs" };
            }
            _ => {}
        }
        k += 1;
    }
    "ok"
}

/// The kinds of the shapes ty's edits are known to break (`arg_shape`, and `vartype-attrobj`: an
/// annotation on another object's attribute, `x.a: T = ...`, which ty rejects; `vartype-classvar`:
/// `model_config: ConfigDict = {...}` in a pydantic model, which ty then rejects as an instance
/// variable overriding the class variable `BaseModel.model_config`).
pub fn is_bad(kind: &str) -> bool {
    kind.starts_with("argname-") || kind == "vartype-attrobj" || kind == "vartype-classvar"
}

/// The variants of a program from its hints.
pub fn variants(file: &str, src: &str, hints: &Value, force: Option<&str>) -> Vec<Variant> {
    let (ids, _) = crate::ren::lex(src);
    let mut by_kind: BTreeMap<&'static str, (Vec<(usize, usize, String)>, Vec<String>)> = BTreeMap::new();
    for h in hints.as_array().into_iter().flatten() {
        let edits: Vec<(usize, usize, String)> = h["edits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| Some((e[0].as_u64()? as usize, e[1].as_u64()? as usize, e[2].as_str()?.to_string())))
            .collect();
        if edits.is_empty() {
            continue;
        }
        let pos = h["pos"].as_u64().unwrap_or(0) as usize;
        if pos > src.len() || !src.is_char_boundary(pos) {
            continue;
        }
        let line = src[..pos].matches('\n').count() + 1;
        let kind = match h["kind"].as_str() {
            Some("arg") => match arg_shape(src, pos) {
                "ok" => "argname",
                "parens" => "argname-parens",
                "varargs" => "argname-varargs",
                _ => "argname-typealiastype",
            },
            Some("type") => {
                let start = src[..pos].rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map_or(0, |p| p + 1);
                let name = &src[start..pos];
                let attr = start > 0 && src.as_bytes()[start - 1] == b'.';
                if name.is_empty() || bindings(src, &ids, name, attr) != 1 {
                    continue;
                }
                let recv = if attr { &src[src[..start - 1].rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map_or(0, |p| p + 1)..start - 1] } else { "" };
                if attr && recv != "self" { "vartype-attrobj" } else if attr { "vartype-attr" } else if in_class_body(src, pos) && name == "model_config" { "vartype-classvar" } else if in_class_body(src, pos) { "vartype-class" } else { "vartype" }
            }
            _ => continue,
        };
        let e = by_kind.entry(kind).or_default();
        e.0.extend(edits);
        e.1.push(format!("line {line}: {}", h["label"].as_str().unwrap_or("?")));
    }
    // At most one of the known-bad shapes, in 1 of TYZ_HINT_BAD_EVERY (250) programs (the parenthesized
    // one breaks most programs that have it and would hide the others).
    let every = std::env::var("TYZ_HINT_BAD_EVERY").ok().and_then(|s| s.parse().ok()).unwrap_or(250u64).max(1);
    let h = incr::hash(src).rotate_left(23);
    let bad: Vec<&'static str> = by_kind.keys().copied().filter(|k| is_bad(k)).collect();
    let pick = match force {
        Some(k) => bad.iter().copied().find(|b| *b == k),
        None => (h % every == 0 && !bad.is_empty()).then(|| bad[(h / every) as usize % bad.len()]),
    };
    by_kind
        .into_iter()
        .filter(|(kind, _)| !is_bad(kind) || Some(*kind) == pick)
        .filter_map(|(kind, (edits, applied))| Some(Variant { file: file.to_string(), kind, text: apply_edits(src, &edits)?, applied }))
        .collect()
}

/// Probe values, each probe's values sorted and deduplicated.
fn norm(obs: &Value) -> BTreeMap<String, Vec<String>> {
    obs.as_object()
        .map(|o| {
            o.iter()
                .map(|(k, xs)| {
                    let mut b: Vec<String> = xs.as_array().into_iter().flatten().map(|x| x.as_str().unwrap_or("").to_string()).collect();
                    b.sort();
                    b.dedup();
                    (k.clone(), b)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Runs the variants under CPython and ty in `work`: program file -> (signature, message), the
/// first finding of each program.
fn judge(work: &Path, vs: &[Variant], obs: &BTreeMap<String, Value>) -> BTreeMap<String, (String, String)> {
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
    // ty's edits import `LiteralString` and others from typing_extensions (not in the stdlib).
    // With the build's virtual environment (pydantic), the real one is installed there.
    if std::env::var_os("VIRTUAL_ENV").is_none() {
        fs::write(rdir.join("typing_extensions.py"), "from typing import *\n").unwrap();
    }
    fs::write(cdir.join("_rt.py"), incr::RT_STUB).unwrap();
    let vname = |v: &Variant| format!("h_{}_{}.py", v.file.trim_end_matches(".py"), v.kind.replace('-', "_"));
    for v in vs {
        fs::write(rdir.join(vname(v)), &v.text).unwrap();
        fs::write(cdir.join(vname(v)), &v.text).unwrap();
        let mut st = VARIANTS.lock().unwrap();
        *st.entry(v.kind).or_default() += 1;
        *st.entry(crate::ren::leak(format!("{}-edits", v.kind))).or_default() += v.applied.len();
    }
    let names: Vec<String> = vs.iter().map(vname).collect();
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
        if found.contains_key(&v.file) {
            continue;
        }
        let n = vname(v);
        let what = format!("applying the {} inlay hint edits of {} ({})", v.kind, v.file, v.applied.join("; "));
        if let Some((sig, msg)) = &fail {
            found.insert(v.file.clone(), (sig.clone(), msg.clone()));
            continue;
        }
        if let Some(d) = diags.get(&n).and_then(|ds| ds.iter().find(|d| d.rule == "panic" || d.severity == "fatal")) {
            found.insert(v.file.clone(), (crate::panic_sig(&d.msg), d.msg.clone()));
            continue;
        }
        let Some(run) = runs.get(&n) else { continue };
        if run["status"] == "timeout" || run["status"] == "crash" {
            continue;
        }
        if run["status"] != "ok" {
            let e = run["error"].as_str().unwrap_or("?");
            found.insert(v.file.clone(), (format!("hint__runtime__{}", v.kind), format!("{what}: the program raises {e}")));
            continue;
        }
        let (got, want) = (norm(&run["obs"]), norm(&obs[&v.file]));
        if got != want {
            let k = got.keys().chain(want.keys()).find(|k| got.get(*k) != want.get(*k)).cloned().unwrap_or_default();
            found.insert(v.file.clone(), (format!("hint__runtime__{}", v.kind), format!("{what}: probe {k}: {:?} vs {:?}", got.get(&k), want.get(&k))));
            continue;
        }
        if let Some(d) = diags.get(&n).and_then(|ds| ds.iter().find(|d| d.severity == "error")) {
            let l = v.text.lines().nth(d.line.saturating_sub(1)).unwrap_or("").trim();
            found.insert(v.file.clone(), (format!("hint__check-{}__{}", d.rule, v.kind), format!("{what}: line {} `{l}`: {}", d.line, d.msg)));
        }
    }
    found
}

/// The oracle on `srcs` (`p_<i>.py` -> program): program file -> (signature, message), or a
/// failure of the whole run. `force`: the known-bad shape variant to check (a repro's).
pub fn check(work: &Path, srcs: &BTreeMap<String, String>, obs: &BTreeMap<String, Value>, force: Option<&str>, panic_sig: impl Fn(&str) -> String + Copy) -> Result<BTreeMap<String, (String, String)>, (String, String)> {
    let indir = work.join("in");
    crate::ren::write_inputs(&indir, srcs);
    let hints = run_tyinc(work, &indir, panic_sig)?;
    let vs: Vec<Variant> = srcs.iter().flat_map(|(f, t)| hints.get(f).map(|h| variants(f, t, h, force)).unwrap_or_default()).collect();
    Ok(judge(work, &vs, obs))
}

/// Finding files: `hint/in/<file>` (the program) and `hint/obs.json` (its probe values).
pub fn finding_files(file: &str, src: &str, obs: &Value) -> Vec<(String, String)> {
    vec![(format!("hint/in/{file}"), src.to_string()), ("hint/obs.json".into(), json!({file: obs}).to_string())]
}

/// `repro` for a hint finding: 1 when the program's hint edits still show `sig`, 0 when not.
pub fn repro(fdir: &Path, sig: &str, panic_sig: impl Fn(&str) -> String + Copy) -> i32 {
    if !Path::new(&incr::tyinc_bin()).is_file() {
        return 2;
    }
    let d = fdir.join("hint");
    let obs: BTreeMap<String, Value> = serde_json::from_str(&fs::read_to_string(d.join("obs.json")).unwrap()).unwrap();
    let mut srcs = BTreeMap::new();
    for e in fs::read_dir(d.join("in")).unwrap().flatten() {
        srcs.insert(e.file_name().to_string_lossy().to_string(), fs::read_to_string(e.path()).unwrap());
    }
    let work: PathBuf = std::env::temp_dir().join(format!("tyz-hint-repro-{}", std::process::id()));
    let force = sig.rsplit("__").next().filter(|k| is_bad(k));
    let res = check(&work, &srcs, &obs, force, panic_sig);
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
