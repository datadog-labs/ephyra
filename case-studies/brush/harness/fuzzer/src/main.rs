// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! brushfuzz: differential (brush vs bash 5.3) + crash oracle over generated scripts.
//!
//!   brushfuzz run <build-dir> <seed> <seconds> <out-dir>   search (see README.md)
//!   brushfuzz repro <build-dir> <case-dir>                 exit 1 = reproduces, 0 = not, 2 = undecided
//!   brushfuzz gen <seed> [count]                           print generated scripts
//!   brushfuzz one <build-dir> <script>                     run both shells on a file, print the verdict

mod exec;
mod gen;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use exec::{Outcome, Shell};
use serde_json::json;

const LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
enum Verdict {
    Pass,
    Skip(&'static str),
    Fail(&'static str, String),
}

fn has(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn show(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    if s.len() > 600 { format!("{}...", &s[..s.char_indices().nth(600).map(|x| x.0).unwrap_or(s.len())]) } else { s.into() }
}

/// The oracles. `brush` is the target, `bash` the reference (bash 5.3, which brush's
/// docs/reference/compatibility.md says it targets). One failure site per signature.
fn classify(brush: &Outcome, bash: &Outcome) -> Verdict {
    // crash oracle: a Rust panic (we build with panic=unwind, so it exits 101 with the message),
    // or death by a signal we did not send.
    if has(&brush.stderr, "panicked at") || brush.code == Some(101) || brush.signal.is_some() {
        return Verdict::Fail("crash__panic", format!("brush crashed: code {:?} signal {:?}\nstderr: {}", brush.code, brush.signal, show(&brush.stderr)));
    }
    if bash.timed_out {
        return Verdict::Skip("ref_timeout");
    }
    if brush.timed_out {
        return Verdict::Fail("hang__timeout", format!("brush did not finish within {LIMIT:?}; bash did (code {:?})", bash.code));
    }
    // brush reports unimplemented features with exit 99 / "not yet implemented"
    // (brush-core/src/error.rs, results.rs:194): a documented gap, not a wrong result.
    if brush.code == Some(99) || has(&brush.stderr, "not yet implemented") || has(&brush.stderr, "not implemented") {
        return Verdict::Skip("unimplemented");
    }
    // brush's printf is uucore's, which refuses field widths above 1_000_000 with this exact error
    // (uucore 0.12 features/format/mod.rs MAX_FORMAT_WIDTH: "to prevent memory allocation panics");
    // compatibility.md:177: "Some advanced `printf` format specifiers behave differently ...,
    // particularly features not supported by the underlying `uucore` library." Generated `%*s`
    // widths like 0X1F104A hit it; bash pads to 2M columns.
    if has(&brush.stderr, "printf: i/o error: formatting width too large") {
        return Verdict::Skip("printf_width_limit");
    }
    if bash.signal.is_some() {
        return Verdict::Skip("ref_signal");
    }
    // bash 5.3 bug (reference, not brush): an unquoted here-string of a case-modified array
    // containing DEL (`a=($'\x7f' ab); cat <<< ${a[@]^^}`) leaks bash's internal quote byte
    // CTLESC (\x01) before the \x7f; `echo ${a[@]^^}` and the quoted form print just \x7f.
    // Same leak, doubled: the unquoted assignment `a=($'\x01'); x=${a[@]/q/}` (also `${a[@]^^}`)
    // stores \x01\x01, while `x="${a[@]/q/}"`, `x=${a[*]/q/}` and `x=${a[0]/q/}` store one \x01.
    // An assignment value is not word-split, so quoting must not change it.
    // Skip only when deleting each such \x01 makes bash's stdout equal brush's.
    let leak = |w: &[u8]| w == b"\x01\x7f" || w == b"\x01\x01";
    if brush.stdout != bash.stdout && bash.stdout.windows(2).any(leak) {
        let mut fixed = Vec::with_capacity(bash.stdout.len());
        let mut i = 0;
        while i < bash.stdout.len() {
            let c = bash.stdout[i];
            match bash.stdout.get(i + 1) {
                Some(&0x7f) if c == 0x01 => {}
                Some(&0x01) if c == 0x01 => {
                    fixed.push(c);
                    i += 1;
                }
                _ => fixed.push(c),
            }
            i += 1;
        }
        if fixed == brush.stdout {
            return Verdict::Skip("ref_ctlesc_leak");
        }
    }
    if brush.stdout != bash.stdout {
        return Verdict::Fail("diff__stdout", format!("stdout differs\n--- bash:\n{}\n--- brush:\n{}\n--- brush stderr:\n{}", show(&bash.stdout), show(&brush.stdout), show(&brush.stderr)));
    }
    if brush.code != bash.code {
        return Verdict::Fail("diff__status", format!("exit status: bash {:?}, brush {:?}\n--- bash stderr:\n{}\n--- brush stderr:\n{}", bash.code, brush.code, show(&bash.stderr), show(&brush.stderr)));
    }
    if brush.files != bash.files {
        let names = |o: &Outcome| o.files.iter().map(|(n, c)| format!("{n}={:?}", String::from_utf8_lossy(c))).collect::<Vec<_>>().join(" ");
        return Verdict::Fail("diff__files", format!("files differ\n--- bash: {}\n--- brush: {}", names(bash), names(brush)));
    }
    // bash warns on stderr (exit 0, same output) when printf's `%b` (or format) meets `\x` with no
    // hex digit or `\u`/`\U` with none; brush's printf is uucore's and expands `%b` silently.
    // compatibility.md:177: "Some advanced `printf` format specifiers behave differently ...,
    // particularly features not supported by the underlying `uucore` library", and
    // tests/cases/compat/builtins/printf.yaml marks "printf %b (backslash escape expansion)"
    // known_failure. Skip only when every bash stderr line is such a warning and brush's is empty.
    if brush.stderr.is_empty() && !bash.stderr.is_empty() && bash_printf_escape_warnings_only(&bash.stderr) {
        return Verdict::Skip("printf_escape_warning");
    }
    // Error message text differs by design (brush prefixes "error:"); compare only whether
    // each shell reported something on stderr.
    if brush.stderr.is_empty() != bash.stderr.is_empty() {
        return Verdict::Fail("diff__stderr", format!("stderr empty: bash {}, brush {}\n--- bash stderr:\n{}\n--- brush stderr:\n{}", bash.stderr.is_empty(), brush.stderr.is_empty(), show(&bash.stderr), show(&brush.stderr)));
    }
    Verdict::Pass
}

fn bash_printf_escape_warnings_only(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr).lines().all(|l| {
        [": printf: missing hex digit for \\x", ": printf: missing unicode digit for \\u", ": printf: missing unicode digit for \\U"]
            .iter()
            .any(|w| l.ends_with(w))
    })
}

/// Feature area of a (shrunk) failing script, appended to `diff__*` signatures so that
/// different causes get different names. Looks only at the generated statements (not the
/// fixed prelude, the variable initialisation in node 0, or the final printf), and names
/// the most specific construct present. A property of the input, not of the build.
fn area(nodes: &[gen::Node]) -> &'static str {
    if nodes.len() <= 1 {
        return "init";
    }
    let text = gen::render(&nodes[1..]);
    area_of(text.strip_prefix(gen::PRELUDE).unwrap_or(&text))
}

fn area_of(text: &str) -> &'static str {
    let text = text.replace("printf '<%s>' \"$v\" \"$w\" \"$x\" \"$n\" \"${a[@]}\" \"${b[@]}\" \"$#\" \"$*\"; echo", "");
    // `echo WORDS | cat` only prints WORDS: a difference there comes from expanding the words,
    // so the trailing `| cat` of a simple echo/printf is not what names the area
    let text: String = text
        .lines()
        .map(|l| {
            let t = l.trim_start();
            match l.strip_suffix(" | cat") {
                Some(s) if (t.starts_with("echo ") || t.starts_with("printf ")) && !s.contains('|') => s,
                _ => l,
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    // round-trip statements (gen.rs roundtrip_func / roundtrip_quote) name their own area: they
    // contain `${v@Q}`-like operators that would otherwise make them param_transform
    if text.contains("declare -f") || text.contains("declare -F") {
        // the construct in the printed function names the area: printing a here-document, a
        // redirected compound command and a case command go wrong in unrelated places
        let heredoc = text.match_indices("<<").any(|(i, _)| {
            let rest = text[i + 2..].trim_start_matches('-').trim_start_matches(['\'', '"']);
            rest.starts_with("EOF")
        });
        if heredoc {
            return "roundtrip_func_heredoc";
        }
        // a subshell body starting with `(` / `((`: brush prints `( (x) )` as `((x))` and parses
        // `( (x) )` as an arithmetic command (upstream #1526). Checked before the redirect area:
        // the generator often wraps the round-trip test itself in a redirected `{ ...; } >f`
        if text.contains("() (\n(") {
            return "roundtrip_func_subshell";
        }
        if gen::compound_redir(&text) {
            return "roundtrip_func_redirect";
        }
        if text.contains("esac") {
            return "roundtrip_func_case";
        }
        return "roundtrip_func";
    }
    if ["@Q}", "@A}", " %q ", "'%q '", "declare -p"].iter().any(|p| text.contains(p)) {
        return "roundtrip_quote";
    }
    // parameter-expansion operators: the char after `${[!#]name[subscript]`
    let mut ops: Vec<&'static str> = vec![];
    let b = text.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'$' && b[i + 1] == b'{' {
            let mut j = i + 2;
            let length = j < b.len() && b[j] == b'#' && j + 1 < b.len() && b[j + 1] != b'}';
            if j < b.len() && (b[j] == b'!' || b[j] == b'#') {
                j += 1;
            }
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'@' && j == i + 2 || b[j] == b'*' && j == i + 2) {
                j += 1;
            }
            if j < b.len() && b[j] == b'[' {
                while j < b.len() && b[j] != b']' {
                    j += 1;
                }
                j += 1;
            }
            let op = match b.get(j) {
                Some(b'/') => "param_patsub",
                Some(b'@') => "param_transform",
                Some(b'^') | Some(b',') => "param_case",
                Some(b'#') | Some(b'%') => "param_trim",
                Some(b':') if matches!(b.get(j + 1), Some(b'-') | Some(b'=') | Some(b'+') | Some(b'?')) => "param_default",
                Some(b':') => "param_substr",
                Some(b'-') | Some(b'=') | Some(b'+') | Some(b'?') => "param_default",
                _ if length => "param_length",
                _ => "",
            };
            if !op.is_empty() {
                ops.push(op);
            }
        }
        i += 1;
    }
    let lines: Vec<&str> = text.lines().collect();
    let any = |f: &dyn Fn(&str) -> bool| lines.iter().any(|l| f(l.trim_start()));
    let starts = |p: &'static str| any(&|l: &str| l.starts_with(p));
    let has = |p: &'static str| text.contains(p);
    // most specific first
    for op in ["param_patsub", "param_transform", "param_case", "param_trim", "param_substr", "param_default"] {
        if ops.contains(&op) {
            return op;
        }
    }
    let is_subscript_assign = |l: &str| (l.starts_with("a[") || l.starts_with("b[")) && (l.contains("]=") || l.contains("]+="));
    let subscript_assign = any(&is_subscript_assign);
    // an element unset leaves an indexed array sparse, where negative subscripts count from
    // the highest index rather than the element count (a cause of its own, upstream brush #1570)
    if subscript_assign && any(&|l: &str| l.starts_with("unset 'a[") || l.starts_with("unset 'b[")) {
        return "array_sparse";
    }
    // so does assigning a[5]/b[5] (arrays start with at most 4 elements), when another
    // subscript assignment (possibly a negative one) follows it
    let gap_assign = |l: &str| l.starts_with("a[5]") || l.starts_with("b[5]");
    if any(&gap_assign) && lines.iter().map(|l| l.trim_start()).filter(|l| is_subscript_assign(l) && !gap_assign(l)).count() > 0 {
        return "array_sparse";
    }
    if subscript_assign || any(&|l: &str| l.starts_with("A[") && l.contains("]=") || l.starts_with("unset '")) {
        return "array_subscript";
    }
    // a break/continue that survives shrinking is the construct at fault, not the loop header around it
    if starts("break") || starts("continue") { return "loop_control"; }
    if starts("mapfile") { return "mapfile"; }
    if has("read -r") { return "read"; }
    if has("<<-") || has("<<EOF") || has("<<'EOF'") { return "heredoc"; }
    if has("<<<") { return "herestring"; }
    if starts("declare") || starts("local ") { return "declare"; }
    if any(&|l: &str| l.starts_with("printf '") && !l.starts_with("printf '<%s>'") && !l.starts_with("printf '%s\\n'")) { return "printf_format"; }
    if starts("case ") { return "case"; }
    if starts("shopt") || starts("set -f") || starts("set +f") || starts("for f in") { return "glob"; }
    if has("[[") || has("test ") || starts("[ ") { return "conditional"; }
    if ops.contains(&"param_length") { return "param_length"; }
    if has("((") || starts("let ") || starts("n+=") { return "arith"; }
    if starts("shift") || starts("set --") { return "positional"; }
    if has("IFS") { return "ifs"; }
    if has(" | ") || has("} |") { return "pipeline"; }
    if has("$(") { return "cmdsub"; }
    if has("() {") { return "function"; }
    if starts("for ") || starts("if ") || starts("(") || starts("{") { return "compound"; }
    // the generator's own `printf '<%s>'` format is not a redirection
    if text.replace("'<%s>'", "").contains('>') { return "redirection"; }
    if has("$'") { return "ansi_c_quote"; }
    if starts("a=(") || starts("b=(") || starts("a+=(") || starts("b+=(") { return "array_assign"; }
    if any(&|l: &str| ["v", "w", "x"].iter().any(|v| l.starts_with(&format!("{v}=")) || l.starts_with(&format!("{v}+=")))) { return "assign"; }
    // word-level expansions that would otherwise all land in `word`
    if has("~") { return "tilde"; }
    if unquoted_glob(&text) { return "glob"; }
    let brace = text.match_indices('{').any(|(i, _)| {
        i.checked_sub(1).map(|p| b[p]) != Some(b'$')
            && text[i..].find('}').is_some_and(|e| text[i..i + e].contains("..") || text[i..i + e].contains(','))
    });
    if brace { return "brace"; }
    "word"
}

/// Whether `text` has a pathname-expansion character (`*`, `?`, `[`) outside quotes and
/// outside `${...}` (`$*`, `$?` are parameters, not globs).
fn unquoted_glob(text: &str) -> bool {
    let b = text.as_bytes();
    let (mut sq, mut dq, mut depth) = (false, false, 0usize);
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if sq {
            sq = c != b'\'';
        } else if c == b'\\' {
            i += 1;
        } else if c == b'"' {
            dq = !dq;
        } else if c == b'\'' && !dq {
            sq = true;
        } else if c == b'$' && b.get(i + 1) == Some(&b'{') {
            depth += 1;
            i += 1;
        } else if c == b'}' && depth > 0 {
            depth -= 1;
        } else if !dq && depth == 0 && matches!(c, b'*' | b'?' | b'[') && (i == 0 || b[i - 1] != b'$') {
            return true;
        }
        i += 1;
    }
    false
}

/// The full signature of a failure on (shrunk) `nodes`: `diff__*` gets the feature area.
fn full_sig(base: &str, nodes: &[gen::Node]) -> String {
    if base.starts_with("diff__") { format!("{base}__{}", area(nodes)) } else { base.to_string() }
}

/// `oracle__kind` of a signature (the part `classify` decides; the area is fixed by the input).
fn base_sig(sig: &str) -> String {
    sig.split("__").take(2).collect::<Vec<_>>().join("__")
}

struct Worker {
    brush: Shell,
    bash: Shell,
    root: PathBuf,
}

impl Worker {
    fn new(build: &Path, root: PathBuf) -> Worker {
        std::fs::create_dir_all(&root).unwrap();
        Worker {
            brush: Shell::brush(build.join("brush").to_str().unwrap()),
            bash: Shell::bash(build.join("bash").to_str().unwrap()),
            root,
        }
    }

    /// Run both shells concurrently, each in its own fresh directory; the script lives outside both.
    fn run_both(&self, script: &str) -> (Outcome, Outcome) {
        let sp = self.root.join("script.sh");
        exec::write_script(&sp, script);
        let (d1, d2) = (self.root.join("t"), self.root.join("r"));
        exec::reset_dir(&d1);
        exec::reset_dir(&d2);
        let a = exec::start(&self.brush, &sp, &d1, LIMIT).expect("spawn brush");
        let b = exec::start(&self.bash, &sp, &d2, LIMIT).expect("spawn bash");
        let ra = a.finish(&d1);
        let rb = b.finish(&d2);
        (ra, rb)
    }

    /// True when bash itself gives different outcomes for `script` across runs (`first` plus `n`
    /// more). Concurrent pipeline stages writing one file are scheduling-dependent in bash too
    /// (`{ { echo 1 | cat; echo X > f; } | cat; } >> f` gave two results over 30 bash 5.3 runs).
    fn bash_flaky(&self, script: &str, first: &Outcome, n: usize) -> bool {
        let sp = self.root.join("script.sh");
        exec::write_script(&sp, script);
        let d = self.root.join("r");
        (0..n).any(|_| {
            exec::reset_dir(&d);
            let o = exec::start(&self.bash, &sp, &d, LIMIT).expect("spawn bash").finish(&d);
            matches!(classify(&o, first), Verdict::Fail(..))
        })
    }

    fn verdict(&self, script: &str) -> (Verdict, Outcome, Outcome) {
        let (a, b) = self.run_both(script);
        (classify(&a, &b), a, b)
    }

    /// Greedy deletion/unwrapping of statements while the same signature keeps failing.
    fn shrink(&self, mut nodes: Vec<gen::Node>, sig: &str, deadline: Instant) -> Vec<gen::Node> {
        let start = Instant::now();
        let mut tries = 0;
        let same = |nodes: &[gen::Node]| -> bool {
            matches!(self.verdict(&gen::render(nodes)).0, Verdict::Fail(s, _) if s == sig)
        };
        for pass in 0..2 {
            let mut k = 1; // node 0 = variable initialisation
            while k < gen::count(&nodes) && tries < 400 && start.elapsed() < Duration::from_secs(40) && Instant::now() < deadline {
                let cand = if pass == 0 { gen::delete_nth(&nodes, k) } else { gen::hoist_nth(&nodes, k) };
                if let Some(c) = cand {
                    tries += 1;
                    if same(&c) {
                        nodes = c;
                        continue;
                    }
                }
                k += 1;
            }
        }
        // one more deletion pass after unwrapping
        let mut k = 1; // node 0 = variable initialisation
        while k < gen::count(&nodes) && tries < 500 && start.elapsed() < Duration::from_secs(50) && Instant::now() < deadline {
            if let Some(c) = gen::delete_nth(&nodes, k) {
                tries += 1;
                if same(&c) {
                    nodes = c;
                    continue;
                }
            }
            k += 1;
        }
        nodes
    }
}

fn mix(seed: u64, i: u64) -> u64 {
    let mut z = seed ^ i.wrapping_mul(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

fn gen_script(seed: u64) -> Vec<gen::Node> {
    let mut src = ephyra::Source::new(seed, 10);
    gen::G::new(&mut src).script()
}

fn outcome_json(o: &Outcome) -> serde_json::Value {
    json!({
        "code": o.code,
        "stdout": String::from_utf8_lossy(&o.stdout),
        "stdout_hex": hex(&o.stdout),
        "stderr_empty": o.stderr.is_empty(),
        "stderr": String::from_utf8_lossy(&o.stderr),
        "files": o.files.iter().map(|(n, c)| json!([n, hex(c)])).collect::<Vec<_>>(),
    })
}

fn outcome_from_json(v: &serde_json::Value) -> Option<Outcome> {
    let files = v["files"].as_array()?.iter().map(|f| Some((f[0].as_str()?.to_string(), unhex(f[1].as_str()?)))).collect::<Option<Vec<_>>>()?;
    Some(Outcome {
        code: v["code"].as_i64().map(|c| c as i32),
        signal: None,
        timed_out: false,
        stdout: unhex(v["stdout_hex"].as_str()?),
        // the text matters only to the stderr skips in classify(); older cases lack it
        stderr: if v["stderr_empty"].as_bool()? { vec![] } else { v["stderr"].as_str().filter(|s| !s.is_empty()).unwrap_or("x").as_bytes().to_vec() },
        files,
    })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap_or(0)).collect()
}

const REPRO_SH: &str = r#"#!/usr/bin/env bash
# Re-runs brush from <build-dir> on script.sh and compares with the stored bash 5.3 outcome
# (expected.json). Exit 1 = the same failure signature reproduces, 0 = it does not, 2 = cannot decide.
cd "$(dirname "$0")" || exit 2
B=${1:?usage: repro.sh <build-dir>}
[ -x "$B/brushfuzz" ] && [ -x "$B/brush" ] || exit 2
exec "$B/brushfuzz" repro "$B" "$PWD"
"#;

fn write_finding(dir: &Path, sig: &str, msg: &str, seed: u64, script: &str, bash: &Outcome, brush: &Outcome) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("script.sh"), script).unwrap();
    let f = json!({"signature": sig, "oracle": sig.split("__").next().unwrap(), "message": msg, "seed": seed});
    std::fs::write(dir.join("finding.json"), serde_json::to_string_pretty(&f).unwrap()).unwrap();
    std::fs::write(dir.join("expected.json"), serde_json::to_string_pretty(&outcome_json(bash)).unwrap()).unwrap();
    std::fs::write(dir.join("brush-observed.json"), serde_json::to_string_pretty(&outcome_json(brush)).unwrap()).unwrap();
    let rp = dir.join("repro.sh");
    std::fs::write(&rp, REPRO_SH).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&rp, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn repro(build: &Path, case: &Path) -> i32 {
    let Ok(fj) = std::fs::read_to_string(case.join("finding.json")) else { return 2 };
    let Ok(ej) = std::fs::read_to_string(case.join("expected.json")) else { return 2 };
    let Ok(script) = std::fs::read_to_string(case.join("script.sh")) else { return 2 };
    let f: serde_json::Value = serde_json::from_str(&fj).unwrap_or_default();
    let sig = f["signature"].as_str().unwrap_or("").to_string();
    let Some(bash) = outcome_from_json(&serde_json::from_str(&ej).unwrap_or_default()) else { return 2 };
    let root = std::env::temp_dir().join(format!("brushfuzz-repro-{}", std::process::id()));
    let w = Worker::new(build, root.clone());
    let sp = root.join("script.sh");
    exec::write_script(&sp, &script);
    // A failure counts as reproduced if 1 of 3 runs shows it (pipelines run concurrently in brush).
    let mut hit = false;
    for _ in 0..3 {
        let d = root.join("t");
        exec::reset_dir(&d);
        let Ok(r) = exec::start(&w.brush, &sp, &d, LIMIT) else { return 2 };
        let o = r.finish(&d);
        if let Verdict::Fail(s, msg) = classify(&o, &bash) {
            if s == base_sig(&sig) {
                eprintln!("{msg}");
                hit = true;
                break;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    if hit { 1 } else { 0 }
}

#[derive(Default)]
struct Stats {
    cases: AtomicU64,
    failures: AtomicU64,
    skips: Mutex<HashMap<&'static str, u64>>,
    fail_sigs: Mutex<HashMap<String, u64>>,
    written: Mutex<HashMap<String, usize>>,
    /// shrinks per base signature (the full signature is only known after shrinking)
    shrunk: Mutex<HashMap<&'static str, usize>>,
}

fn write_stats(out: &Path, st: &Stats, start: Instant) {
    let cases = st.cases.load(Ordering::Relaxed);
    let mut j = json!({
        "cases": cases,
        "failures": st.failures.load(Ordering::Relaxed),
        "elapsed_s": start.elapsed().as_secs_f64(),
        "cases_per_s": cases as f64 / start.elapsed().as_secs_f64().max(0.001),
    });
    for (k, v) in st.skips.lock().unwrap().iter() {
        j[format!("skip_{k}")] = json!(v);
    }
    for (k, v) in st.fail_sigs.lock().unwrap().iter() {
        j[format!("fail_{k}")] = json!(v);
    }
    let tmp = out.join("stats.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&j).unwrap()).unwrap();
    std::fs::rename(&tmp, out.join("stats.json")).unwrap();
}

fn run(build: PathBuf, seed: u64, secs: u64, out: PathBuf) {
    std::fs::create_dir_all(out.join("findings")).unwrap();
    let start = Instant::now();
    let deadline = start + Duration::from_secs(secs);
    let st = Arc::new(Stats::default());
    write_stats(&out, &st, start);
    let threads: usize = std::env::var("BRUSHFUZZ_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    let next = Arc::new(AtomicU64::new(0));
    // findings written per signature (the contract caps at 3; raise only for triage)
    let cap: usize = std::env::var("BRUSHFUZZ_CAP").ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    let done = Arc::new(AtomicBool::new(false));
    let scratch = std::env::temp_dir().join(format!("brushfuzz-{}-{seed}", std::process::id()));
    let mut hs = vec![];
    for t in 0..threads {
        let (build, out, st, next, scratch) = (build.clone(), out.clone(), st.clone(), next.clone(), scratch.clone());
        hs.push(std::thread::spawn(move || {
            let w = Worker::new(&build, scratch.join(format!("w{t}")));
            while Instant::now() < deadline {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let cseed = mix(seed, i);
                let nodes = gen_script(cseed);
                let script = gen::render(&nodes);
                let (v, _, b0) = w.verdict(&script);
                st.cases.fetch_add(1, Ordering::Relaxed);
                // a reference that disagrees with itself is no oracle (failures only, so it is cheap)
                let v = match v {
                    Verdict::Fail(base, _) if base.starts_with("diff__") && w.bash_flaky(&script, &b0, 8) => {
                        Verdict::Skip("ref_nondeterministic")
                    }
                    v => v,
                };
                match v {
                    Verdict::Pass => {}
                    Verdict::Skip(r) => *st.skips.lock().unwrap().entry(r).or_default() += 1,
                    Verdict::Fail(base, _) => {
                        st.failures.fetch_add(1, Ordering::Relaxed);
                        {
                            let mut sh = st.shrunk.lock().unwrap();
                            let e = sh.entry(base).or_default();
                            // enough shrinks to fill several areas; then only count
                            if *e >= cap * 12 {
                                *st.fail_sigs.lock().unwrap().entry(base.to_string()).or_default() += 1;
                                continue;
                            }
                            *e += 1;
                        }
                        let small = w.shrink(nodes.clone(), base, deadline);
                        let text = gen::render(&small);
                        let (v2, a2, b2) = w.verdict(&text);
                        let (sig, text, msg, a, b) = match v2 {
                            Verdict::Fail(s, m) if s == base => (full_sig(base, &small), text, m, a2, b2),
                            _ => {
                                let (v3, a3, b3) = w.verdict(&script);
                                let m = if let Verdict::Fail(_, m) = v3 { m } else { "flaky".into() };
                                (full_sig(base, &nodes), script.clone(), m, a3, b3)
                            }
                        };
                        *st.fail_sigs.lock().unwrap().entry(sig.clone()).or_default() += 1;
                        let n = {
                            let mut wr = st.written.lock().unwrap();
                            let e = wr.entry(sig.clone()).or_default();
                            if *e >= cap { continue; }
                            *e += 1;
                            *e
                        };
                        let dir = out.join("findings").join(&sig).join(n.to_string());
                        write_finding(&dir, &sig, &msg, cseed, &text, &b, &a);
                    }
                }
            }
        }));
    }
    {
        let (out, st) = (out.clone(), st.clone());
        let done = done.clone();
        hs.push(std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(2));
                write_stats(&out, &st, start);
            }
        }));
    }
    let writer = hs.pop().unwrap();
    for h in hs {
        let _ = h.join();
    }
    done.store(true, Ordering::Relaxed);
    let _ = writer.join();
    write_stats(&out, &st, start);
    let _ = std::fs::remove_dir_all(&scratch);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("run") => run(a[2].clone().into(), a[3].parse().expect("seed"), a[4].parse().expect("seconds"), a[5].clone().into()),
        Some("repro") => std::process::exit(repro(Path::new(&a[2]), Path::new(&a[3]))),
        Some("gen") => {
            let seed: u64 = a[2].parse().unwrap();
            let n: u64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
            for i in 0..n {
                println!("##### case {i}\n{}", gen::render(&gen_script(mix(seed, i))));
            }
        }
        Some("one") => {
            let script = std::fs::read_to_string(&a[3]).unwrap();
            let root = std::env::temp_dir().join(format!("brushfuzz-one-{}", std::process::id()));
            let w = Worker::new(Path::new(&a[2]), root.clone());
            let (v, br, ba) = w.verdict(&script);
            println!("{v:?}\n--- brush ({:?}):\n{}\n--- brush stderr:\n{}\n--- bash ({:?}):\n{}\n--- bash stderr:\n{}", br.code, show(&br.stdout), show(&br.stderr), ba.code, show(&ba.stdout), show(&ba.stderr));
            println!("bash nondeterministic: {}", w.bash_flaky(&script, &ba, 8));
            let _ = std::fs::remove_dir_all(&root);
        }
        _ => {
            eprintln!("usage: brushfuzz run|repro|gen|one ...");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{area_of, classify, exec::Outcome, unquoted_glob, Verdict};

    #[test]
    fn bash_ctlesc_leak_is_skipped_narrowly() {
        let o = |s: &[u8]| Outcome { code: Some(0), stdout: s.to_vec(), ..Default::default() };
        assert!(matches!(classify(&o(b"\x7f AB\n"), &o(b"\x01\x7f AB\n")), Verdict::Skip("ref_ctlesc_leak")));
        // brush dropping the \x7f too is still a failure
        assert!(matches!(classify(&o(b" AB\n"), &o(b"\x01\x7f AB\n")), Verdict::Fail("diff__stdout", _)));
        assert!(matches!(classify(&o(b"x\n"), &o(b"\x7f\n")), Verdict::Fail("diff__stdout", _)));
        assert!(matches!(classify(&o(b"<\x01 -3>\n"), &o(b"<\x01\x01 -3>\n")), Verdict::Skip("ref_ctlesc_leak")));
        // brush dropping the \x01 entirely is still a failure
        assert!(matches!(classify(&o(b"< -3>\n"), &o(b"<\x01\x01 -3>\n")), Verdict::Fail("diff__stdout", _)));
    }

    #[test]
    fn printf_escape_warning_is_skipped_narrowly() {
        let o = |out: &[u8], err: &[u8]| Outcome { code: Some(0), stdout: out.to_vec(), stderr: err.to_vec(), ..Default::default() };
        let warn = b"../script.sh: line 3: printf: missing hex digit for \\x\n";
        assert!(matches!(classify(&o(b"a\\x\n", b""), &o(b"a\\x\n", warn)), Verdict::Skip("printf_escape_warning")));
        // any other bash diagnostic alongside it still counts
        let mixed = b"../script.sh: line 3: printf: missing hex digit for \\x\n../script.sh: line 4: x: unbound variable\n";
        assert!(matches!(classify(&o(b"a\\x\n", b""), &o(b"a\\x\n", mixed)), Verdict::Fail("diff__stderr", _)));
        // a different output is still a stdout failure
        assert!(matches!(classify(&o(b"\\x\n", b""), &o(b"\\u\n", warn)), Verdict::Fail("diff__stdout", _)));
    }

    #[test]
    fn echo_into_cat_is_not_pipeline() {
        assert_eq!(area_of("echo \"${a[@]}\" ~{01..3} | cat\n"), "tilde");
        assert_eq!(area_of("echo \"$*\".* | cat\n"), "glob");
        assert_eq!(area_of("{ echo 1; } | cat\n"), "pipeline");
        assert_eq!(area_of("x=1 | cat\necho \"$x\"\n"), "pipeline");
        assert_eq!(area_of("echo a | tr a b | cat\n"), "pipeline");
    }

    #[test]
    fn glob_detection() {
        assert!(unquoted_glob("printf '<%s>' $x ''.*; echo"));
        assert!(unquoted_glob("echo sub/\"\".*"));
        assert!(!unquoted_glob("printf '<%s>' \"$*\" $* $?; echo"));
        assert!(!unquoted_glob("echo \"${v/[ab]/x}\" '*'"));
        assert!(!unquoted_glob("echo ${a[@]} ${v##*.}"));
    }

    #[test]
    fn roundtrip_areas() {
        assert_eq!(area_of("eval \"$(declare -f f)\"; echo \"rc=$?\"\n"), "roundtrip_func");
        assert_eq!(area_of("f() {\ncat <<-'EOF'\n\tx $((1 << 2))\nEOF\n}\neval \"$(declare -f f)\"\n"), "roundtrip_func_heredoc");
        assert_eq!(area_of("f() { echo $((1 << 2)); }\neval \"$(declare -f f)\"\n"), "roundtrip_func");
        assert_eq!(area_of("g() {\n{\n:\n} 1>&2 2>/dev/null; cat f\n}\neval \"$(declare -f g)\"\n"), "roundtrip_func_redirect");
        assert_eq!(area_of("f() (\n(( n = ((~ 010)) ))\n)\n[[ \"$(declare -f f)\" == x ]]\n"), "roundtrip_func_subshell");
        assert_eq!(area_of("f() (\necho x\n)\neval \"$(declare -f f)\"\n"), "roundtrip_func");
        assert_eq!(area_of("g() (\n(( n = (((i))) ))\n)\n{\n[[ \"$(declare -f g)\" == x ]]; echo \"fp=$?\"\n} >'g.log' 2>/dev/null\n"), "roundtrip_func_subshell");
        assert_eq!(area_of("f() (\ncase x in\nx) :\n;;\nesac\n)\neval \"$(declare -f f)\"\n"), "roundtrip_func_case");
        assert_eq!(area_of("v='a;b'; eval \"y=${v@Q}\"; printf '<%s>' \"$y\"; echo\n"), "roundtrip_quote");
        assert_eq!(area_of("printf -v y '%q ' \"${a[@]}\"; eval \"a=($y)\"\n"), "roundtrip_quote");
        assert_eq!(area_of("echo ${v@U}\n"), "param_transform");
    }

    #[test]
    fn sparse_array_area() {
        use crate::gen::Node;
        let nodes = |stmts: &[&str]| {
            let mut v = vec![Node(vec![crate::gen::Seg::Text("a=(x y z)".into())])];
            v.extend(stmts.iter().map(|s| Node(vec![crate::gen::Seg::Text(s.to_string())])));
            v
        };
        assert_eq!(super::area(&nodes(&["set -- ", "a[5]+=?.txt", "a[n]+=\"${v}\""])), "array_sparse");
        assert_eq!(super::area(&nodes(&["unset 'a[1]'", "a[n]+=x"])), "array_sparse");
        assert_eq!(super::area(&nodes(&["a[5]=x", "echo ?"])), "array_subscript");
        assert_eq!(super::area(&nodes(&["a[1]+=x", "a[n]=y"])), "array_subscript");
    }
}
