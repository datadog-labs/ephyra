// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The `incr__` oracle: ty's incremental checking (watch mode, the language server) must agree
//! with a fresh check. `bin/tyinc` (tyinc/) checks step 0 of a project in one database, applies
//! the edits of step 1 and re-checks, checks step 1 again in a fresh database, then reverts to
//! step 0 and re-checks. Step 0 is the batch's check files with a `reveal_type` after every probe,
//! plus per program a module `u_<i>.py` that imports and reveals every top-level name of
//! `c_<i>.py`, so cross-module invalidation is compared too. Step 1 edits each program
//! (`edit`): replace a statement by `pass`, swap a builtin type or a literal, rename a top-level
//! definition, drop a return or variable annotation, cut a line short as if typing, or insert a
//! line at the top.
//!
//! Signatures: `incr__edit__<rule>` (incremental step 1 differs from fresh step 1) and
//! `incr__revert__<rule>` (incremental step 2 differs from step 0), with `<rule>` the rule of the
//! first differing diagnostic and `-import` when it is in the importing module;
//! `crash__panic__*` for a panic (a diagnostic, an IDE request, or one that kills tyinc);
//! `crash__exit__tyinc_*` when tyinc dies otherwise.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const TY_TOML: &str = "[environment]\npython-version = \"3.13\"\n";
pub const RT_STUB: &str = "def _probe(k: int, v: object) -> None: ...\n";

pub fn tyinc_bin() -> String {
    std::env::var("TYZ_TYINC").unwrap_or_else(|_| {
        let ty = std::env::var("TYZ_TY").unwrap_or_default();
        Path::new(&ty).with_file_name("tyinc").to_string_lossy().to_string()
    })
}

/// Whether the oracle runs: TYZ_INCR=0 turns it off, and it needs bin/tyinc.
pub fn enabled() -> bool {
    std::env::var("TYZ_INCR").map(|v| v != "0").unwrap_or(true) && Path::new(&tyinc_bin()).is_file()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

pub fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

fn indent(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Replaces whole-word occurrences of `from` in `line` (strings included).
fn replace_word(line: &str, from: &str, to: &str, only_first: bool) -> String {
    let mut out = String::new();
    let mut rest = line;
    let mut done = false;
    while let Some(p) = rest.find(from) {
        out.push_str(&rest[..p]);
        let before_ok = out.chars().last().is_none_or(|c| !is_ident(c));
        let after_ok = rest[p + from.len()..].chars().next().is_none_or(|c| !is_ident(c));
        if before_ok && after_ok && !done {
            out.push_str(to);
            done = only_first;
        } else {
            out.push_str(from);
        }
        rest = &rest[p + from.len()..];
    }
    out.push_str(rest);
    out
}

/// Top-level names a program defines (functions, classes, aliases, variables), without `_` ones.
pub fn top_names(src: &str) -> Vec<String> {
    let mut names = Vec::new();
    for l in src.lines() {
        if l.starts_with(' ') || l.starts_with('@') || l.starts_with('#') {
            continue;
        }
        let l = l.strip_prefix("async ").unwrap_or(l);
        let rest = l.strip_prefix("def ").or(l.strip_prefix("class ")).or(l.strip_prefix("type ")).unwrap_or(l);
        let name: String = rest.chars().take_while(|&c| is_ident(c)).collect();
        if name.is_empty() || name.starts_with('_') || name.chars().next().unwrap().is_ascii_digit() {
            continue;
        }
        let after = rest[name.len()..].trim_start();
        let defn = rest.len() != l.len() || after.starts_with(':') || (after.starts_with('=') && !after.starts_with("=="));
        if defn && !["if", "for", "while", "try", "with", "match", "import", "from", "else", "return"].contains(&name.as_str()) && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// The importing module for program `i`.
pub fn importer(i: usize, src: &str) -> String {
    let names = top_names(src);
    let mut out = format!("import typing as _t\nimport c_{i} as _m\n");
    if !names.is_empty() {
        out.push_str(&format!("from c_{i} import {}\n", names.join(", ")));
    }
    for n in &names {
        out.push_str(&format!("_t.reveal_type({n})\n_t.reveal_type(_m.{n})\n"));
    }
    // Members of the classes, through an instance and the class, and a subclass of each class
    // (errors for final classes and enums with members are fine: only agreement is checked).
    for (c, members) in class_members(src) {
        for m in &members {
            out.push_str(&format!("_t.reveal_type(_t.cast({c}, None).{m})\n_t.reveal_type({c}.{m})\n"));
        }
        out.push_str(&format!("class _S_{c}({c}):\n    pass\n_t.reveal_type(_S_{c}.__mro__)\n"));
        if let Some(m) = members.first() {
            out.push_str(&format!("_t.reveal_type(_t.cast(_S_{c}, None).{m})\n"));
        }
    }
    out
}

/// Top-level classes and the names their bodies define (`name:`, `name =`, `def name(`).
pub fn class_members(src: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut cur: Option<usize> = None;
    for l in src.lines() {
        if !l.starts_with(' ') {
            cur = None;
            if let Some(rest) = l.strip_prefix("class ") {
                let name: String = rest.chars().take_while(|&c| is_ident(c)).collect();
                if !name.is_empty() && !name.starts_with('_') {
                    out.push((name, vec![]));
                    cur = Some(out.len() - 1);
                }
            }
            continue;
        }
        let (Some(ci), Some(body)) = (cur, l.strip_prefix("    ")) else { continue };
        if body.starts_with(' ') {
            continue;
        }
        let body = body.strip_prefix("async ").unwrap_or(body);
        let rest = body.strip_prefix("def ").unwrap_or(body);
        let name: String = rest.chars().take_while(|&c| is_ident(c)).collect();
        let after = rest[name.len()..].trim_start();
        let is_def = rest.len() != body.len() || after.starts_with(':') || (after.starts_with('=') && !after.starts_with("=="));
        if is_def && !name.is_empty() && !(name.starts_with("__") && name != "__init__" && name != "__call__") && !["pass", "return", "if", "for"].contains(&name.as_str()) && !out[ci].1.contains(&name) {
            out[ci].1.push(name);
        }
    }
    out
}

/// A `reveal_type` after every probe of a check file.
pub fn with_reveals(check: &str) -> String {
    let mut out = String::new();
    for l in check.lines() {
        out.push_str(l);
        out.push('\n');
        let t = l.trim_start();
        if let Some(rest) = t.strip_prefix("_probe(")
            && let Some((_, e)) = rest.split_once(", ")
            && let Some(e) = e.strip_suffix(')')
        {
            out.push_str(&format!("{}_t.reveal_type({e})\n", &l[..indent(l)]));
        }
    }
    out
}

/// One to three random edits of a program (deterministic in `seed`).
pub fn edit(src: &str, seed: u64) -> String {
    edit_with(src, seed, true)
}

/// `edit`; without `typing`, a line is never cut short (the result usually still parses).
pub fn edit_with(src: &str, seed: u64, typing: bool) -> String {
    let mut r = Rng(seed);
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
    let n = 1 + r.below(3);
    for _ in 0..n {
        if lines.is_empty() {
            break;
        }
        match r.below(10) {
            // A statement or definition (with its block) becomes `pass`.
            0 | 1 => {
                let cands: Vec<usize> = (0..lines.len())
                    .filter(|&i| {
                        let t = lines[i].trim_start();
                        !t.is_empty() && !t.starts_with('@') && !(i > 0 && lines[i - 1].trim_start().starts_with('@'))
                            && !["else", "elif", "except", "finally", "case"].iter().any(|k| t.starts_with(k))
                    })
                    .collect();
                if cands.is_empty() {
                    continue;
                }
                let i = cands[r.below(cands.len())];
                let ind = indent(&lines[i]);
                let mut j = i + 1;
                while j < lines.len() && (lines[j].trim().is_empty() || indent(&lines[j]) > ind) {
                    j += 1;
                }
                lines.splice(i..j, [format!("{}pass", " ".repeat(ind))]);
            }
            // A builtin type in an annotation or call becomes another one.
            2 => {
                const TYS: [&str; 6] = ["int", "str", "bool", "float", "bytes", "None"];
                let cands: Vec<(usize, &str)> = (0..lines.len())
                    .flat_map(|i| TYS.iter().map(move |t| (i, *t)))
                    .filter(|(i, t)| replace_word(&lines[*i], t, "\u{0}", true) != lines[*i])
                    .collect();
                if cands.is_empty() {
                    continue;
                }
                let (i, from) = cands[r.below(cands.len())];
                let to = TYS[r.below(TYS.len())];
                lines[i] = replace_word(&lines[i], from, to, true);
            }
            // An integer literal changes, or a string literal gains a character.
            3 => {
                let cands: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].chars().any(|c| c.is_ascii_digit() || c == '"')).collect();
                if cands.is_empty() {
                    continue;
                }
                let i = cands[r.below(cands.len())];
                let l = &lines[i];
                let digit = l.char_indices().find(|&(p, c)| c.is_ascii_digit() && l[..p].chars().last().map(|b| !is_ident(b)).unwrap_or(true));
                lines[i] = match digit {
                    Some((p, _)) if r.below(2) == 0 || !l.contains('"') => {
                        let end = l[p..].find(|c: char| !c.is_ascii_digit()).map(|e| p + e).unwrap_or(l.len());
                        format!("{}{}{}", &l[..p], ["0", "1", "7", "-1", "100"][r.below(5)], &l[end..])
                    }
                    _ => l.replacen('"', "\"z", 1),
                };
            }
            // A top-level definition is renamed everywhere in this file.
            4 => {
                let names = top_names(&lines.join("\n"));
                if names.is_empty() {
                    continue;
                }
                let from = names[r.below(names.len())].clone();
                let to = format!("{from}_r");
                for l in lines.iter_mut() {
                    *l = replace_word(l, &from, &to, false);
                }
            }
            // A return annotation goes (ty infers the return type, recursion makes cycles), or a
            // variable annotation does.
            5 | 6 => {
                let cands: Vec<usize> = (0..lines.len())
                    .filter(|&i| {
                        let t = lines[i].trim_start();
                        (t.starts_with("def ") || t.starts_with("async def ")) && t.contains(") -> ") && t.ends_with(':')
                            || (!t.starts_with("def ") && !t.starts_with("class ") && t.split_once(": ").is_some_and(|(a, b)| a.chars().all(is_ident) && b.contains(" = ")))
                    })
                    .collect();
                if cands.is_empty() {
                    continue;
                }
                let i = cands[r.below(cands.len())];
                let l = lines[i].clone();
                let t = l.trim_start();
                let ind = &l[..indent(&l)];
                lines[i] = if t.ends_with(':') && t.contains(") -> ") {
                    let p = l.rfind(") -> ").unwrap();
                    format!("{}):", &l[..p])
                } else {
                    let (a, b) = t.split_once(": ").unwrap();
                    format!("{ind}{a} = {}", b.split_once(" = ").unwrap().1)
                };
            }
            // Typing in progress: a line is cut at some column and gets a token that leaves it
            // unfinished (the state a language server sees between keystrokes).
            7 | 8 if typing => {
                let cands: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].trim().len() > 3).collect();
                if cands.is_empty() {
                    continue;
                }
                let i = cands[r.below(cands.len())];
                let l = &lines[i];
                let start = indent(l);
                let mut cut = start + 1 + r.below(l.len() - start);
                while !l.is_char_boundary(cut) {
                    cut -= 1;
                }
                let tail = ["", ".", "(", "[", ",", " if ", " lambda", ": ", " = ", "[int", "{", "\"", " match", "*"][r.below(14)];
                lines[i] = format!("{}{}", &l[..cut], tail);
            }
            // A line at the top shifts every offset and adds a name.
            _ => {
                let at = lines.iter().position(|l| !l.starts_with("from __future__")).unwrap_or(0);
                lines.insert(at, "_tyz_pad: int = 0  # inserted".to_string());
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

pub fn write_step(dir: &Path, files: &[(String, String)]) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
    for (n, s) in files {
        fs::write(dir.join(n), s).unwrap();
    }
}

/// One check of tyinc: (step, mode, file -> diagnostics).
pub type Check = (u64, String, BTreeMap<String, Vec<String>>);

/// Edited steps after step 0; tyinc runs s0, +s1 .. +s<EDITS>, s0.
pub const EDITS: usize = 3;

/// Runs tyinc on the steps under `steps` (`s0` .. `s<EDITS>`). Err((signature, message)) when it dies.
pub fn run(work: &Path, steps: &Path, panic_sig: impl Fn(&str) -> String) -> Result<Vec<Check>, (String, String)> {
    let t0 = Instant::now();
    let mut cmd = Command::new(tyinc_bin());
    cmd.arg(work.join("proj")).arg(steps.join("s0"));
    for k in 1..=EDITS {
        cmd.arg(format!("+{}", steps.join(format!("s{k}")).display()));
    }
    let mut child = cmd
        .arg(steps.join("s0"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("tyinc");
    let timeout = Duration::from_secs(180);
    let mut stdout = String::new();
    let mut stderr = String::new();
    // Drain stdout on a thread so a large output cannot block the child.
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
    stdout.push_str(&ho.join().unwrap_or_default());
    stderr.push_str(&he.join().unwrap_or_default());
    match status {
        None => return Err(("crash__hang__tyinc".into(), format!("tyinc ran past {timeout:?}"))),
        Some(s) if !s.success() => {
            let tail: String = stderr.lines().filter(|l| !l.trim().is_empty()).take(30).collect::<Vec<_>>().join("\n");
            // A panic outside the per-file checks (tyinc's hook prints `Panicked at <loc> in tyinc: <msg>`).
            if let Some(p) = stderr.lines().find(|l| l.starts_with("Panicked at ")) {
                return Err((panic_sig(p), tail));
            }
            return Err((format!("crash__exit__tyinc_{}", s.code().map(|c| c.to_string()).unwrap_or("signal".into())), tail));
        }
        _ => {}
    }
    let mut checks = Vec::new();
    for l in stdout.lines() {
        if let Ok(v) = serde_json::from_str::<Value>(l) {
            let diags: BTreeMap<String, Vec<String>> = serde_json::from_value(v["diags"].clone()).unwrap_or_default();
            checks.push((v["step"].as_u64().unwrap_or(99), v["mode"].as_str().unwrap_or("?").to_string(), diags));
        }
    }
    if checks.iter().filter(|c| c.1 != "ide").count() != 2 * EDITS + 2 {
        return Err(("crash__exit__tyinc_output".into(), format!("{} checks in tyinc output", checks.len())));
    }
    Ok(checks)
}

fn rule_of(d: &str) -> String {
    d.split_once('[').and_then(|(_, r)| r.split_once(']')).map(|(r, _)| r.to_string()).unwrap_or_else(|| "unknown".into())
}

/// Per file: the first disagreement, as (signature, message). `only` restricts the files.
pub fn compare(checks: &[Check], only: Option<&BTreeSet<String>>, panic_sig: impl Fn(&str) -> String) -> BTreeMap<String, (String, String)> {
    let empty = BTreeMap::new();
    let get = |step: usize, mode: &str| checks.iter().find(|c| c.0 == step as u64 && c.1 == mode).map(|c| &c.2).unwrap_or(&empty);
    let mut files: BTreeSet<&String> = BTreeSet::new();
    for c in checks {
        files.extend(c.2.keys());
    }
    // (kind, step, a, b, a's name, b's name): every edited step against a fresh check, then the
    // revert against step 0.
    let mut pairs = Vec::new();
    for k in 1..=EDITS {
        pairs.push(("edit", k, get(k, "inc"), get(k, "fresh"), "incremental", "fresh"));
    }
    pairs.push(("revert", EDITS + 1, get(EDITS + 1, "inc"), get(0, "inc"), "after revert", "before edits"));
    let none = Vec::new();
    // A check that panicked (ty reports the panic under `-`, without a file) drops the panicking
    // file's diagnostics; that difference is the panic's, not a separate incremental bug.
    let panicked = |step: usize| checks.iter().any(|c| c.0 == step as u64 && c.2.values().flatten().any(|d| rule_of(d) == "panic"));
    let mut out = BTreeMap::new();
    for f in files {
        if only.is_some_and(|o| !o.contains(f)) {
            continue;
        }
        // A panic anywhere is a crash, whichever check it is in.
        if let Some(p) = checks.iter().flat_map(|c| c.2.get(f).unwrap_or(&none)).find(|d| rule_of(d) == "panic") {
            let msg = p.split_once("] ").map(|x| x.1).unwrap_or(p);
            out.insert(f.clone(), (panic_sig(msg), format!("{f}: {p}")));
            continue;
        }
        let imp = if f.starts_with('c') { "" } else { "-import" };
        for &(kind, k, a, b, an, bn) in &pairs {
            let (da, db) = (a.get(f).unwrap_or(&none), b.get(f).unwrap_or(&none));
            if da == db || panicked(k) || (kind == "revert" && panicked(0)) {
                continue;
            }
            let sa: BTreeSet<&String> = da.iter().collect();
            let sb: BTreeSet<&String> = db.iter().collect();
            let only_a: Vec<&&String> = sa.difference(&sb).collect();
            let only_b: Vec<&&String> = sb.difference(&sa).collect();
            let first = only_a.first().or(only_b.first()).map(|d| d.as_str()).unwrap_or("[order] order differs");
            let msg = format!(
                "{f}: {kind} at step {k}: only {an}: {}; only {bn}: {}",
                only_a.iter().take(3).map(|d| d.as_str()).collect::<Vec<_>>().join(" | "),
                only_b.iter().take(3).map(|d| d.as_str()).collect::<Vec<_>>().join(" | ")
            );
            out.insert(f.clone(), (format!("incr__{kind}__{}{imp}", rule_of(first)), msg));
            break;
        }
    }
    out
}

/// The files of program `i` at each step 0..=EDITS, from its check-file maker `check` (which
/// takes a program source): s1 edits the program, s2 edits it again and makes `u_<i>` also reveal
/// the names through `m_<i>`, which does not exist yet; s3 creates `m_<i>.py` re-exporting
/// `c_<i>` with a star import (and edits once more).
pub fn program_steps(i: usize, src: &str, check: impl Fn(&str) -> String) -> Vec<Vec<(String, String)>> {
    let u = importer(i, src);
    let names = top_names(src);
    let mut u2 = u.clone();
    u2.push_str(&format!("import m_{i} as _mm\n"));
    for n in &names {
        u2.push_str(&format!("_t.reveal_type(_mm.{n})\n"));
    }
    let e1 = edit(src, hash(src));
    let e2 = edit(&e1, hash(&e1) ^ 1);
    let e3 = edit(&e2, hash(&e2) ^ 2);
    let (c, uf, mf) = (format!("c_{i}.py"), format!("u_{i}.py"), format!("m_{i}.py"));
    let m = format!("from c_{i} import *\n");
    vec![
        vec![(c.clone(), with_reveals(&check(src))), (uf.clone(), u.clone())],
        vec![(c.clone(), with_reveals(&check(&e1))), (uf.clone(), u)],
        vec![(c.clone(), with_reveals(&check(&e2))), (uf.clone(), u2.clone())],
        vec![(c, with_reveals(&check(&e3))), (uf, u2), (mf, m)],
    ]
}

/// The files every step has.
pub fn common() -> Vec<(String, String)> {
    vec![("_rt.py".to_string(), RT_STUB.to_string()), ("ty.toml".to_string(), TY_TOML.to_string())]
}

/// Writes steps (one file list per step) as `<dir>/s<k>`.
pub fn write_steps(dir: &Path, steps: &[Vec<(String, String)>]) {
    for (k, files) in steps.iter().enumerate() {
        write_step(&dir.join(format!("s{k}")), files);
    }
}

/// `repro` for an incr finding: `<fdir>/incr/{s0..,only.txt}`. 1 when any of three runs
/// shows `sig` (thread scheduling may matter), 0 when none does, 2 when tyinc is missing.
pub fn repro(fdir: &Path, sig: &str, panic_sig: impl Fn(&str) -> String + Copy) -> i32 {
    if !Path::new(&tyinc_bin()).is_file() {
        return 2;
    }
    let only: BTreeSet<String> = fs::read_to_string(fdir.join("incr/only.txt")).unwrap_or_default().lines().map(str::to_string).collect();
    let work: PathBuf = std::env::temp_dir().join(format!("tyz-incr-repro-{}", std::process::id()));
    let mut code = 0;
    for _ in 0..3 {
        match run(&work, &fdir.join("incr"), panic_sig) {
            Err((s, m)) => {
                println!("{s}: {m}");
                if s == sig {
                    code = 1;
                    break;
                }
            }
            Ok(checks) => {
                let cmp = compare(&checks, if only.is_empty() { None } else { Some(&only) }, panic_sig);
                println!("{cmp:?}");
                if cmp.values().any(|(s, _)| s == sig) {
                    code = 1;
                    break;
                }
            }
        }
    }
    let _ = fs::remove_dir_all(&work);
    code
}
