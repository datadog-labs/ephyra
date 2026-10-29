// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The `rt__` oracle: ty's suppression round trips. Each program that passed the other oracles is
//! made ill-typed (`incr::edit` twice), some of its lines get `ty: ignore`/`type: ignore`
//! comments, and `reflow` spreads lines over several physical lines (a break after a comma inside
//! brackets, a backslash continuation, a string literal turned into a triple-quoted one across a
//! line break), so diagnostics span lines and suppression comments have awkward places to go.
//!
//! - `--add-ignore` ("Adds `ty: ignore` comments to suppress all rule diagnostics"): afterwards a
//!   fresh check reports no lint diagnostic in a file without syntax errors, and no unused
//!   ignore comment that was not already unused before; the program's AST is unchanged; the
//!   diagnostics `--add-ignore` reports as remaining are those of the fresh check.
//! - `--update-baseline` then `--baseline`: a baseline written from a check suppresses every lint
//!   diagnostic of that same check (ty_project/src/baseline.rs: "When a diagnostic matches a
//!   baseline entry, ty lowers its severity to hint. The CLI hides these hints"), also after
//!   lines are added at the top of every file ("Adding the import changes the line number ... but
//!   preserves all four values"); updating the baseline again without changes rewrites it as is.
//! - `--fix` on the program with extra ignore comments: no internal error (a fix that introduces
//!   a syntax error, fixes that don't converge), no unused ignore comment left, the file still
//!   parses, the remaining diagnostics it reports are those of a fresh check.
//!
//! Signatures `rt__<mode>__<rule>`: mode `ignore`, `ignore-ast`, `ignore-report`, `baseline`,
//! `baseline-shift`, `baseline-update`, `fix`, `fix-ast`, `fix-report`; rule the first offending
//! diagnostic's rule (`internal-syntax`/`internal-converge` for ty's own internal errors).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::incr;

/// Unused ignore comments left by `--fix` because their removal is an unsafe fix.
pub static UNSAFE_FIX_SKIPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

const TY_TOML: &str = "[environment]\npython-version = \"3.13\"\n[rules]\n\
possibly-unresolved-reference = \"warn\"\npossibly-missing-attribute = \"warn\"\npossibly-missing-import = \"warn\"\n\
division-by-zero = \"warn\"\nmissing-override-decorator = \"warn\"\nredundant-condition-strict = \"warn\"\n\
truthiness-test-of-none-union = \"warn\"\nunsound-assignment = \"warn\"\nunsound-return-statement = \"warn\"\n\
missing-type-argument = \"warn\"\nblanket-ignore-comment = \"warn\"\n";

const AST_PY: &str = include_str!("../py/astcmp.py");
const CASTS_PY: &str = include_str!("../py/casts.py");

/// Diagnostic ids that are not lints (ruff_db's `DiagnosticId`): never suppressed or baselined.
const NOT_LINT: &[&str] = &[
    "panic", "io", "invalid-syntax", "revealed-type", "unknown-rule", "invalid-glob", "invalid-script-metadata",
    "uv-metadata", "empty-include", "unnecessary-overrides-section", "useless-overrides-section", "deprecated-setting",
    "unsupported-python-version", "unformatted", "invalid-cli-option", "invalid-baseline", "preview-feature", "internal-error",
];

pub fn enabled() -> bool {
    std::env::var("TYZ_RT").map(|v| v != "0").unwrap_or(true)
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

/// Byte offsets in `l` outside string literals and before any comment, with the bracket depth
/// there. `None` when the line has a quote the scanner cannot follow (an unterminated string).
fn code_positions(l: &str) -> Option<Vec<(usize, char, usize)>> {
    let b: Vec<(usize, char)> = l.char_indices().collect();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut k = 0;
    while k < b.len() {
        let (p, c) = b[k];
        match c {
            '#' => break,
            '"' | '\'' => {
                let triple = k + 2 < b.len() && b[k + 1].1 == c && b[k + 2].1 == c;
                if triple {
                    return None;
                }
                let mut j = k + 1;
                while j < b.len() && b[j].1 != c {
                    if b[j].1 == '\\' {
                        j += 1;
                    }
                    j += 1;
                }
                if j >= b.len() {
                    return None;
                }
                k = j + 1;
                continue;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        out.push((p, c, depth));
        k += 1;
    }
    Some(out)
}

/// Spreads some lines over several physical lines without changing what they mean (apart from
/// a newline inside a string literal).
pub fn reflow(src: &str, seed: u64) -> String {
    let mut r = Rng(seed);
    // A backslash continuation right before a trailing comment hits known bugs in `--fix` and
    // `--add-ignore` (fixes that never converge or break the syntax, unused ignores left
    // behind) in most files that have one: only one file in eight may get them, so that the
    // rest of the oracle is not drowned out.
    let awkward = (seed >> 7) % 8 == 0;
    let mut out = String::new();
    let mut in_continuation = false;
    for l in src.lines() {
        let cont = in_continuation;
        in_continuation = l.trim_end().ends_with('\\');
        let ind = l.len() - l.trim_start().len();
        let pos = code_positions(l);
        if cont || pos.is_none() || l.trim().is_empty() || r.below(4) != 0 {
            out.push_str(l);
            out.push('\n');
            continue;
        }
        let pos = pos.unwrap();
        let pad = " ".repeat(ind + 4);
        match r.below(3) {
            // A break after a comma inside brackets.
            0 => {
                let commas: Vec<usize> = pos.iter().filter(|&&(_, c, d)| c == ',' && d > 0).map(|&(p, _, _)| p).collect();
                if let Some(&p) = commas.get(r.below(commas.len().max(1))) {
                    out.push_str(&format!("{}\n{pad}{}\n", &l[..=p], l[p + 1..].trim_start()));
                    continue;
                }
            }
            // A backslash continuation at a space outside brackets.
            1 => {
                // Rarely right before a trailing comment, which then sits alone on the next line.
                let code_end = pos.iter().rev().find(|&&(_, c, _)| c != ' ').map(|&(p, _, _)| p).unwrap_or(0);
                let before_comment = r.below(20) == 0 && awkward;
                let spaces: Vec<usize> = pos.iter().filter(|&&(p, c, d)| c == ' ' && d == 0 && p > ind && (p < code_end || before_comment)).map(|&(p, _, _)| p).collect();
                if let Some(&p) = spaces.get(r.below(spaces.len().max(1))) {
                    out.push_str(&format!("{} \\\n{pad}{}\n", &l[..p], l[p + 1..].trim_start()));
                    continue;
                }
            }
            // A plain "..." literal becomes a triple-quoted one with a line break inside.
            _ => {
                let prefix_ok = |p: usize| l[..p].chars().last().is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '"');
                if let Some(p) = l.find('"').filter(|&p| prefix_ok(p))
                    && let Some(q) = l[p + 1..].find('"').map(|q| p + 1 + q)
                    && !l[p..q].contains('\\')
                    && code_positions(&l[..p]).is_some_and(|v| v.len() == l[..p].chars().count())
                {
                    let inner = &l[p + 1..q];
                    let cut = (0..=inner.len()).filter(|&c| inner.is_char_boundary(c)).nth(r.below(inner.chars().count() + 1)).unwrap_or(0);
                    out.push_str(&format!("{}\"\"\"{}\n{}\"\"\"{}\n", &l[..p], &inner[..cut], &inner[cut..], &l[q + 1..]));
                    continue;
                }
            }
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// Appends suppression comments to about one line in `rate`.
pub fn add_comments(src: &str, seed: u64, rate: usize) -> String {
    const C: [&str; 8] = [
        "  # ty: ignore[invalid-argument-type]",
        "  # ty: ignore",
        "  # type: ignore",
        "  # ty: ignore[unresolved-reference, invalid-assignment]",
        "  # ty:ignore[possibly-unresolved-reference]",
        "  # noqa: E501  # ty: ignore[invalid-return-type]",
        "  # type: ignore[attr-defined]",
        "  # ty: ignore[unresolved-attribute]  # trailing",
    ];
    let mut r = Rng(seed);
    let mut out = String::new();
    for l in src.lines() {
        out.push_str(l);
        if !l.trim().is_empty() && !l.contains('#') && !l.trim_end().ends_with('\\') && code_positions(l).is_some() && r.below(rate) == 0 {
            out.push_str(C[r.below(C.len())]);
        }
        out.push('\n');
    }
    out
}

/// `py/casts.py`: `cast(T, e)` around some expressions of `src` (unchanged when it does not parse).
pub fn casts(src: &str, seed: u64) -> String {
    let script = std::env::temp_dir().join(format!("tyz-casts-{}.py", std::process::id()));
    if fs::read_to_string(&script).ok().as_deref() != Some(CASTS_PY) {
        let tmp = script.with_extension(format!("{}.tmp", seed));
        fs::write(&tmp, CASTS_PY).unwrap();
        fs::rename(&tmp, &script).unwrap();
    }
    let child = Command::new(crate::py_bin())
        .arg("-I")
        .arg(&script)
        .arg("-")
        .arg((seed % 1_000_000_007).to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return src.to_string() };
    let mut stdin = child.stdin.take().unwrap();
    let input = src.to_string();
    let w = std::thread::spawn(move || {
        let _ = std::io::Write::write_all(&mut stdin, input.as_bytes());
    });
    let out = child.wait_with_output();
    let _ = w.join();
    match out {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => src.to_string(),
    }
}

/// The two inputs of program `i`: `r_<i>.py` (add-ignore, baseline) and `f_<i>.py` (fix).
pub fn inputs(i: usize, src: &str) -> Vec<(String, String)> {
    let h = incr::hash(src);
    // One program in eight may come out syntactically broken (files `--add-ignore` and `--fix`
    // skip, but the baseline records).
    let typing = h % 8 == 0;
    let e1 = incr::edit_with(src, h ^ 0x5254, typing);
    let e2 = incr::edit_with(&e1, h ^ 0x5255, typing);
    let r = texture(&reflow(&add_comments(&e2, h ^ 7, 12), h ^ 8), h ^ 13);
    let f = texture(&reflow(&add_comments(&casts(&incr::edit_with(src, h ^ 9, typing), h ^ 12), h ^ 10, 5), h ^ 11), h ^ 14);
    vec![(format!("r_{i}.py"), r), (format!("f_{i}.py"), f)]
}

/// The file's text as editors leave it, independently per property: non-ASCII text in some
/// string literals and comments (offsets in bytes, chars and UTF-16 units differ), CRLF line
/// endings, a UTF-8 BOM, tabs for indentation, no newline at the end. Python reads all of them
/// the same way (apart from the strings' contents).
pub fn texture(src: &str, seed: u64) -> String {
    let mut r = Rng(seed);
    let unicode = r.below(3) == 0;
    let crlf = r.below(4) == 0;
    let bom = r.below(6) == 0;
    let tabs = r.below(8) == 0;
    let no_eol = r.below(8) == 0;
    const U: [&str; 4] = ["é", "日本", "😀", "ß€"];
    let mut lines: Vec<String> = Vec::new();
    let mut in_continuation = false;
    let mut in_triple = false;
    for l in src.lines() {
        let cont = in_continuation;
        in_continuation = l.trim_end().ends_with('\\');
        // Lines inside or opening a triple-quoted string (reflow makes them) keep their text.
        let triples = l.matches("\"\"\"").count();
        let inside = in_triple;
        let keep = in_triple || triples > 0;
        if triples % 2 == 1 {
            in_triple = !in_triple;
        }
        let mut l = l.to_string();
        // (Not on lines with prefixed strings: f-strings may nest quotes, bytes must stay ASCII.)
        let prefixed = l.as_bytes().windows(2).any(|w| w[1] == b'"' && w[0].is_ascii_alphabetic());
        if unicode && !keep && !prefixed && !l.contains('\'') && !l.contains('\\') && let Some(pos) = code_positions(&l) {
            // After an opening quote of a plain "..." literal (a code position is never inside
            // a string: a quote right after one opens a literal when the count so far is even).
            let quotes: Vec<usize> = l.char_indices().filter(|&(_, c)| c == '"').map(|(p, _)| p).collect();
            let code_end = pos.last().map(|&(p, _, _)| p + 1).unwrap_or(0);
            if let Some(&q) = quotes.iter().step_by(2).find(|&&q| q < code_end && r.below(2) == 0)
                && l[..q].chars().last().is_none_or(|c| !c.is_ascii_alphanumeric())
                && l[q + 1..].starts_with(|c: char| c != '"')
            {
                l.insert_str(q + 1, U[r.below(U.len())]);
            }
            if r.below(10) == 0 && !l.contains('#') && !l.trim_end().ends_with('\\') && !l.trim().is_empty() {
                l.push_str(&format!("  # {}", U[r.below(U.len())]));
            }
        }
        if tabs && !cont && !inside {
            let ind = l.len() - l.trim_start_matches(' ').len();
            if ind % 4 == 0 {
                l = format!("{}{}", "\t".repeat(ind / 4), &l[ind..]);
            }
        }
        lines.push(l);
    }
    let nl = if crlf { "\r\n" } else { "\n" };
    let mut out = lines.join(nl);
    if !no_eol {
        out.push_str(nl);
    }
    if bom {
        out.insert(0, '\u{feff}');
    }
    out
}

#[derive(Clone, Debug)]
struct D {
    file: String,
    line: usize,
    sev: String,
    rule: String,
    text: String,
}

fn parse(out: &str) -> Vec<D> {
    let mut v = Vec::new();
    for l in out.lines() {
        // `path:line:col: sev[rule] msg`, or `path: sev[rule] msg` for a file-level one
        // (ty's internal errors about fixes), or without a path.
        let mut it = l.splitn(4, ':');
        let (path, line, rest) = match (it.next(), it.next(), it.next(), it.next()) {
            (Some(p), Some(n), Some(_), Some(rest)) if n.parse::<usize>().is_ok() => (p, n.parse().unwrap(), rest),
            _ => match l.split_once(": ") {
                Some((p, rest)) if !p.contains(' ') && !p.contains('[') => (p, 0, rest),
                _ => ("", 0, l),
            },
        };
        let Some((sev, rest)) = rest.trim_start().split_once('[') else { continue };
        if !["error", "warning", "info", "fatal", "hint"].contains(&sev) {
            continue;
        }
        let Some((rule, _)) = rest.split_once("] ") else { continue };
        let file = Path::new(path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        v.push(D { file, line, sev: sev.to_string(), rule: rule.to_string(), text: l.to_string() });
    }
    v
}

fn is_lint(d: &D) -> bool {
    !NOT_LINT.contains(&d.rule.as_str())
}

/// One `ty check` in `dir`. Err((signature, message)) on a hang or a crash exit.
fn ty(dir: &Path, extra: &[&str]) -> Result<String, (String, String)> {
    let t0 = Instant::now();
    let mut child = Command::new(crate::ty_bin())
        .args(["check", "--output-format", "concise", "--no-progress", "--project", "."])
        .args(extra)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ty");
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
    let mode = extra.first().map(|a| a.trim_start_matches('-')).unwrap_or("check");
    match status {
        None => Err((format!("crash__hang__ty_{mode}"), format!("ty check {extra:?} ran past {timeout:?}"))),
        // 101 with a fatal diagnostic in the output (a panic, an internal error) is for the
        // caller to sort out.
        Some(s) if s.code() == Some(101) && parse(&stdout).iter().any(|d| d.sev == "fatal") => Ok(stdout),
        // A Rust panic outside the checks (`thread '..' panicked at <loc>:` on stderr, then an
        // abort when it is on a rayon thread): named after the location and the mode.
        Some(s) if s.code().is_none_or(|c| c > 1) && stderr.contains("panicked at ") => {
            let loc = stderr.split("panicked at ").nth(1).and_then(|r| r.split_whitespace().next()).unwrap_or("unknown:0:");
            let mut parts = loc.trim_end_matches(':').rsplitn(3, ':').collect::<Vec<_>>();
            parts.reverse();
            let file = parts.first().map(|f| f.rsplit('/').next().unwrap_or(f).trim_end_matches(".rs")).unwrap_or("unknown");
            let line = parts.get(1).copied().unwrap_or("0");
            Err((format!("crash__panic__{file}_{line}_{mode}"), stderr.lines().filter(|l| !l.starts_with("WARN ")).take(40).collect::<Vec<_>>().join("\n")))
        }
        Some(s) if s.code().is_none_or(|c| c > 1) => {
            Err((format!("crash__exit__ty_{mode}_{}", s.code().map(|c| c.to_string()).unwrap_or("signal".into())), stderr.lines().filter(|l| !l.starts_with("WARN ")).take(40).collect::<Vec<_>>().join("\n")))
        }
        _ => Ok(stdout),
    }
}

/// Per file: does it parse before / after, and is its AST the same (py/astcmp.py).
fn ast_compare(work: &Path, pairs: &[(String, String)], uncast: bool) -> BTreeMap<String, (bool, bool, bool)> {
    let script = work.join("astcmp.py");
    fs::write(&script, AST_PY).unwrap();
    let mut cmd = Command::new(crate::py_bin());
    cmd.arg("-I").arg(&script);
    if uncast {
        cmd.arg("--uncast");
    }
    for (a, b) in pairs {
        cmd.arg(a).arg(b);
    }
    let out = cmd.stderr(Stdio::null()).output().expect("python");
    let mut m = BTreeMap::new();
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() == 4 {
            let name = Path::new(f[0]).file_name().unwrap().to_string_lossy().to_string();
            m.insert(name, (f[1] == "1", f[2] == "1", f[3] == "1"));
        }
    }
    m
}

fn copy_in(src: &Path, dst: &Path, prefix: &str) {
    let _ = fs::remove_dir_all(dst);
    fs::create_dir_all(dst).unwrap();
    fs::write(dst.join("_rt.py"), incr::RT_STUB).unwrap();
    fs::write(dst.join("ty.toml"), TY_TOML).unwrap();
    for e in fs::read_dir(src).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with(prefix) {
            fs::copy(e.path(), dst.join(&n)).unwrap();
        }
    }
}

fn internal_sig(d: &D) -> &'static str {
    if d.text.contains("syntax error") {
        "internal-syntax"
    } else if d.text.contains("converge") {
        "internal-converge"
    } else {
        "internal-error"
    }
}

/// Runs the oracle on the input files under `work/in` (`r_*.py`, `f_*.py`). Returns per input
/// file the first finding, (signature, message); the key `-` for one that is not a file's.
pub fn check(work: &Path, panic_sig: impl Fn(&str) -> String) -> BTreeMap<String, (String, String)> {
    let mut found: BTreeMap<String, (String, String)> = BTreeMap::new();
    fn add(found: &mut BTreeMap<String, (String, String)>, f: &str, sig: String, msg: String) {
        found.entry(f.to_string()).or_insert((sig, msg));
    }
    let inp = work.join("in");
    // Panics are crashes whichever step shows them.
    let panics = |out: &str, found: &mut BTreeMap<String, (String, String)>| {
        for d in parse(out) {
            if d.rule == "panic" {
                let msg = d.text.split_once("] ").map(|x| x.1).unwrap_or(&d.text).to_string();
                add(found, if d.file.is_empty() { "-" } else { &d.file }, panic_sig(&msg), d.text.clone());
            }
        }
    };
    macro_rules! run {
        ($dir:expr, $args:expr) => {
            match ty($dir, $args) {
                Ok(o) => {
                    panics(&o, &mut found);
                    o
                }
                Err((s, m)) => {
                    add(&mut found, "-", s, m);
                    return found;
                }
            }
        };
    }
    let mut names: Vec<String> = fs::read_dir(&inp).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();

    // --add-ignore
    let a = work.join("a");
    copy_in(&inp, &a, "r_");
    let a0 = parse(&run!(&a, &[]));
    let a1 = parse(&run!(&a, &["--add-ignore"]));
    let syntax_bad = |ds: &[D], f: &str| ds.iter().any(|d| d.file == f && d.rule == "invalid-syntax");
    let a2 = parse(&run!(&a, &[]));
    let after1: BTreeMap<String, String> = names.iter().filter(|n| n.starts_with("r_")).map(|n| (n.clone(), fs::read_to_string(a.join(n)).unwrap_or_default())).collect();
    // "Adds `ty: ignore` comments to suppress all rule diagnostics": once they are all
    // suppressed, a second run has nothing to add.
    run!(&a, &["--add-ignore"]);
    for (n, t1) in &after1 {
        let t2 = fs::read_to_string(a.join(n)).unwrap_or_default();
        if *t1 != t2 && !syntax_bad(&a2, n) {
            let k = t1.lines().zip(t2.lines()).position(|(x, y)| x != y).unwrap_or(0);
            add(&mut found, n, "rt__ignore-idem__changed".into(), format!("{n}: a second --add-ignore changed line {}: {:?} -> {:?}", k + 1, t1.lines().nth(k).unwrap_or(""), t2.lines().nth(k).unwrap_or("")));
        }
    }
    let rs: Vec<&String> = names.iter().filter(|n| n.starts_with("r_")).collect();
    let ast_a = ast_compare(work, &rs.iter().map(|n| (inp.join(n).display().to_string(), a.join(n).display().to_string())).collect::<Vec<_>>(), false);
    // (An existing comment that gets the new codes appended changes its message from "Unused
    // directive" to "Unused directive: '<code>'": compare lines only.)
    let unused_before: BTreeSet<(String, usize)> =
        a0.iter().filter(|d| d.rule.starts_with("unused-") && d.rule.ends_with("ignore-comment")).map(|d| (d.file.clone(), d.line)).collect();
    for d in &a1 {
        if d.rule == "internal-error" || d.sev == "fatal" && d.rule != "panic" {
            add(&mut found, &d.file, format!("rt__ignore__{}", internal_sig(d)), d.text.clone());
        }
    }
    for n in &rs {
        let n = n.as_str();
        if let Some(&(before, after, same)) = ast_a.get(n) {
            if before && !after {
                add(&mut found, n, "rt__ignore-ast__syntax".into(), format!("{n}: parses before --add-ignore, not after"));
            } else if before && !same {
                add(&mut found, n, "rt__ignore-ast__changed".into(), format!("{n}: --add-ignore changed the AST"));
            }
        }
        if syntax_bad(&a0, n) || syntax_bad(&a2, n) {
            continue;
        }
        let left = a2.iter().find(|d| {
            d.file == n
                && is_lint(d)
                && (d.sev == "error" || d.sev == "warning")
                && !(d.rule.starts_with("unused-") && d.rule.ends_with("ignore-comment")
                    && unused_before.contains(&(d.file.clone(), d.line)))
        });
        if let Some(d) = left {
            add(&mut found, n, format!("rt__ignore__{}", d.rule), format!("after --add-ignore: {}", d.text));
        }
        let rep: BTreeSet<&String> = a1.iter().filter(|d| d.file == n && d.rule != "internal-error").map(|d| &d.text).collect();
        let fresh: BTreeSet<&String> = a2.iter().filter(|d| d.file == n).map(|d| &d.text).collect();
        if rep != fresh {
            let first = rep.symmetric_difference(&fresh).next().unwrap();
            let rule = a1.iter().chain(&a2).find(|d| &d.text == *first).map(|d| d.rule.clone()).unwrap_or_default();
            add(&mut found, n, format!("rt__ignore-report__{rule}"), format!("--add-ignore reports {:?}, a fresh check {:?}", rep.difference(&fresh).take(3).collect::<Vec<_>>(), fresh.difference(&rep).take(3).collect::<Vec<_>>()));
        }
    }

    // --update-baseline / --baseline
    let b = work.join("b");
    copy_in(&inp, &b, "r_");
    let b0 = parse(&run!(&b, &[]));
    // Writing a baseline when there is none yet panics now and then (a sort over diagnostics
    // that include the span-less "cannot load the baseline" one): one batch in four starts
    // without one, the others from an empty baseline.
    let contents: String = names.iter().map(|n| fs::read_to_string(inp.join(n)).unwrap_or_default()).collect();
    if incr::hash(&contents) % 4 != 0 {
        fs::write(b.join("base.json"), "{\n  \"version\": 0,\n  \"files\": {}\n}\n").unwrap();
    }
    run!(&b, &["--update-baseline", "--baseline", "base.json"]);
    let base1 = fs::read_to_string(b.join("base.json")).unwrap_or_default();
    let b2 = parse(&run!(&b, &["--baseline", "base.json"]));
    let lint_left = |ds: &[D], f: &str| ds.iter().find(|d| d.file == f && is_lint(d) && (d.sev == "error" || d.sev == "warning")).cloned();
    for n in &rs {
        if let Some(d) = lint_left(&b2, n) {
            add(&mut found, n, format!("rt__baseline__{}", d.rule), format!("with the baseline just written: {}", d.text));
        }
    }
    run!(&b, &["--update-baseline", "--baseline", "base.json"]);
    let base2 = fs::read_to_string(b.join("base.json")).unwrap_or_default();
    if base1 != base2 {
        let v1: serde_json::Value = serde_json::from_str(&base1).unwrap_or_default();
        let v2: serde_json::Value = serde_json::from_str(&base2).unwrap_or_default();
        let f = rs.iter().find(|n| v1["files"][n.as_str()] != v2["files"][n.as_str()]).map(|n| n.as_str()).unwrap_or("-");
        add(&mut found, f, "rt__baseline-update__changed".into(), format!("{f}: updating an unchanged project's baseline changed it: {} -> {}", v1["files"][f], v2["files"][f]));
    }
    for n in &rs {
        let t = fs::read_to_string(b.join(n.as_str())).unwrap_or_default();
        // (After a BOM: one in the middle of a file is a stray character.)
        let (bom, t) = t.strip_prefix('\u{feff}').map(|t| ("\u{feff}", t)).unwrap_or(("", &t));
        let nl = if t.contains("\r\n") { "\r\n" } else { "\n" };
        fs::write(b.join(n.as_str()), format!("{bom}# shifted{nl}{nl}{t}")).unwrap();
    }
    let b3 = parse(&run!(&b, &["--baseline", "base.json"]));
    for n in &rs {
        // Only lint diagnostics the unshifted check also had: a shift may not create any, but
        // keep the oracle about matching.
        if let Some(d) = lint_left(&b3, n)
            && b0.iter().any(|o| o.file == d.file && o.rule == d.rule && o.line + 2 == d.line)
        {
            add(&mut found, n, format!("rt__baseline-shift__{}", d.rule), format!("after two lines at the top, with the baseline: {}", d.text));
        }
    }

    // --fix
    let f = work.join("f");
    copy_in(&inp, &f, "f_");
    let f1 = parse(&run!(&f, &["--fix"]));
    let f2 = parse(&run!(&f, &[]));
    let fs_: Vec<&String> = names.iter().filter(|n| n.starts_with("f_")).collect();
    let ast_f = ast_compare(work, &fs_.iter().map(|n| (inp.join(n).display().to_string(), f.join(n).display().to_string())).collect::<Vec<_>>(), true);
    for d in &f1 {
        if d.rule == "internal-error" || d.sev == "fatal" && d.rule != "panic" {
            add(&mut found, &d.file, format!("rt__fix__{}", internal_sig(d)), d.text.clone());
        }
    }
    for n in &fs_ {
        let n = n.as_str();
        if let Some(&(before, after, same)) = ast_f.get(n) {
            if before && !after {
                add(&mut found, n, "rt__fix-ast__syntax".into(), format!("{n}: parses before --fix, not after"));
            } else if before && !same {
                // The only safe fixes are removing unused ignore comments and redundant casts
                // ("Remove the redundant `cast`"): the AST may lose casts, nothing else.
                add(&mut found, n, "rt__fix-ast__changed".into(), format!("{n}: --fix changed the AST other than by removing casts"));
            }
        }
        if syntax_bad(&f2, n) {
            continue;
        }
        // ty marks removing an own-line ignore comment that is followed by another comment
        // (`    # ty: ignore[x]  # trailing`) as an unsafe fix, which `--fix` does not apply
        // (crates/ty_python_semantic/src/suppression/unused.rs: "Removing `# ty: ignore` from
        // `# ty: ignore # fmt: off` would promote `# fmt: off` to the primary own-line comment.").
        let fixed_text = fs::read_to_string(f.join(n)).unwrap_or_default();
        let unsafe_fix = |d: &D| {
            let unsafe_ = d.line > 0
                && fixed_text.lines().nth(d.line - 1).is_some_and(|l| {
                    let l = l.trim_start();
                    l.starts_with('#') && l[1..].split_once('#').is_some_and(|(first, _)| first.contains("ignore"))
                });
            if unsafe_ {
                UNSAFE_FIX_SKIPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            unsafe_
        };
        if let Some(d) = f2.iter().find(|d| d.file == n && d.rule == "unused-ignore-comment" && !unsafe_fix(d)) {
            add(&mut found, n, "rt__fix__unused-ignore-comment".into(), format!("after --fix: {}", d.text));
        }
        let rep: BTreeSet<&String> = f1.iter().filter(|d| d.file == n && d.rule != "internal-error").map(|d| &d.text).collect();
        let fresh: BTreeSet<&String> = f2.iter().filter(|d| d.file == n).map(|d| &d.text).collect();
        if rep != fresh {
            let first = rep.symmetric_difference(&fresh).next().unwrap();
            let rule = f1.iter().chain(&f2).find(|d| &d.text == *first).map(|d| d.rule.clone()).unwrap_or_default();
            add(&mut found, n, format!("rt__fix-report__{rule}"), format!("--fix reports {:?}, a fresh check {:?}", rep.difference(&fresh).take(3).collect::<Vec<_>>(), fresh.difference(&rep).take(3).collect::<Vec<_>>()));
        }
    }
    found
}

/// Writes the inputs of programs `progs` (index, source) to `work/in`.
pub fn write_inputs(work: &Path, files: &[(String, String)]) {
    let inp = work.join("in");
    let _ = fs::remove_dir_all(work);
    fs::create_dir_all(&inp).unwrap();
    for (n, t) in files {
        fs::write(inp.join(n), t).unwrap();
    }
}

/// `repro` for an rt finding: `<fdir>/rt/in`. 1 when the oracle shows `sig` again (on the files
/// of `rt/only.txt` when it exists), 0 when not.
pub fn repro(fdir: &Path, sig: &str, panic_sig: impl Fn(&str) -> String) -> i32 {
    let only: BTreeSet<String> = fs::read_to_string(fdir.join("rt/only.txt")).unwrap_or_default().lines().map(str::to_string).collect();
    let work = std::env::temp_dir().join(format!("tyz-rt-repro-{}", std::process::id()));
    let files: Vec<(String, String)> = fs::read_dir(fdir.join("rt/in"))
        .map(|d| d.flatten().map(|e| (e.file_name().to_string_lossy().to_string(), fs::read_to_string(e.path()).unwrap_or_default())).collect())
        .unwrap_or_default();
    // Crashes can depend on thread scheduling (a sort over diagnostics collected in parallel):
    // up to five runs for them.
    let runs = if sig.starts_with("crash__") { 5 } else { 1 };
    let mut code = 0;
    for _ in 0..runs {
        write_inputs(&work, &files);
        let found = check(&work, &panic_sig);
        println!("{found:?}");
        if found.iter().any(|(f, (s, _))| s == sig && (only.is_empty() || only.contains(f) || f == "-")) {
            code = 1;
            break;
        }
    }
    // A panic in `--update-baseline` without a baseline file yet (one in tens of runs): that one
    // `ty check` alone, many times.
    if code == 0 && sig.starts_with("crash__") && sig.ends_with("_update-baseline") {
        let b = work.join("b");
        for _ in 0..60 {
            copy_in(&work.join("in"), &b, "r_");
            if matches!(ty(&b, &["--update-baseline", "--baseline", "base.json"]), Err((s, _)) if s == sig) {
                code = 1;
                break;
            }
        }
    }
    let _ = fs::remove_dir_all(&work);
    code
}
