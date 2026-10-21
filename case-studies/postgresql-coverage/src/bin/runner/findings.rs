//! Findings on disk, under `D/findings/`:
//!
//! - `<signature>/` for oracle and model failures ([`signature`]: the oracle,
//!   then the message's first line, which names the variant, with numbers
//!   masked), so one directory gathers one kind of failure; up to
//!   [`EXAMPLES`] examples each, every one a directory `w<K>-<n>` with
//!   `message.txt`, `input.txt` (the shrunk input's Debug), `script.sql`
//!   (the shrunk input as a psql script: setup, statements, queries;
//!   [`crate::script`]), `state.txt` (the unshrunk draw's seed and size,
//!   what `runner replay` rebuilds) and `example.json`; `count` is how many
//!   were seen in all, and `finding.json` says where the signature was first
//!   seen and holds its `status`, `open` until triage says otherwise.
//! - `crashes/<signature>/<time>/` for server crashes ([`crash_signature`]:
//!   the assertion, PANIC or signal from the server log): `crash.txt` (the
//!   log excerpt and every worker's input in flight; the culprits are the
//!   workers whose backend pid the log names) and per worker in
//!   flight `w<K>.state.txt` and `w<K>.sql` (its input rebuilt from its
//!   seed); `count` and `finding.json` as above.
//! - `timeouts/`: the first few inputs per worker that hit the statement
//!   timeout outside the oracles' own counted skips (unshrunk examples).
//! - `hangs/`, `memory/`, `worker-crashes/`, `errors/`, `generator-panics/`:
//!   one record per event (`w<K>-<time>.txt`) with the input in flight as
//!   the worker's state file gave it.

use std::{
    fs,
    path::{Path, PathBuf},
};

use ephyra::{Gen, Source};
use ephyra_postgresql::probes::case_queries;

use crate::{
    now, pairs,
    records::{self, esc, Identity},
    script, Input,
};

/// Examples kept per signature.
pub const EXAMPLES: usize = 20;

/// Folders of `findings/` that are not failure signatures.
pub const RECORD_FOLDERS: [&str; 7] = ["crashes", "hangs", "memory", "worker-crashes", "errors", "generator-panics", "timeouts"];

/// `text` with every run of digits as `N` and anything but letters, digits
/// and `-` as `_`.
fn fold(text: &str) -> String {
    let mut out = String::new();
    let mut last_digit = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            if !last_digit {
                out.push('N');
            }
            last_digit = true;
            continue;
        }
        last_digit = false;
        out.push(if ch.is_ascii_alphanumeric() || ch == '-' { ch } else { '_' });
    }
    out
}

/// The SQLSTATE a message ends with (`pg::message` writes `msg (XXXXX)`).
fn sqlstate(message: &str) -> Option<&str> {
    let t = message.trim_end();
    let code = t.strip_suffix(')')?.rsplit('(').next()?;
    (code.len() == 5 && code.chars().all(|c| c.is_ascii_alphanumeric())).then_some(code)
}

/// The signature of a failure: the oracle and the message's first line
/// (`plans-enable_hashjoin: results differ`), numbers masked, made safe as
/// a directory name. A first line that holds SQL (a statement of the case
/// or an original query that fails) is replaced by its kind, the
/// statement's first word and the error's SQLSTATE
/// (`model-statement-UPDATE-XX000`), so that inputs do not each make their
/// own signature.
pub fn signature(oracle: &str, message: &str) -> String {
    let first = message.lines().next().unwrap_or("");
    let kind = if let Some(rest) = first.strip_prefix("statement ") {
        // `statement <k>: <sql>: <error>` or `statement <k> of the case: ..`
        let sql = rest.split_once(": ").map_or("", |(_, s)| s);
        let word = sql.split_whitespace().next().unwrap_or("");
        format!("statement-{}-{}", fold(word), sqlstate(message).unwrap_or("error"))
    } else if let Some(sql) = first.strip_prefix("the original query fails: ") {
        let word = sql.split_whitespace().next().unwrap_or("");
        format!("original-fails-{}-{}", fold(word), sqlstate(message).unwrap_or("error"))
    } else {
        fold(first)
    };
    let mut out = format!("{oracle}-{kind}");
    out.truncate(120);
    out
}

/// The signature of a server crash, from the new part of its log: the first
/// `TRAP:` line (an assertion: `trap-failed_Assert__x____File___y___Line_N`,
/// the PID cut off), else the first `PANIC:` message, else the signal a
/// server process was terminated by (`signal-N`), else `server-lost`.
pub fn crash_signature(log: &str) -> String {
    let mut out = if let Some(l) = log.lines().find_map(|l| l.split_once("TRAP: ").map(|(_, r)| r)) {
        format!("trap-{}", fold(l.split(", PID").next().unwrap_or(l)))
    } else if let Some(l) = log.lines().find_map(|l| l.split_once("PANIC:  ").map(|(_, r)| r)) {
        format!("panic-{}", fold(l))
    } else if let Some(l) = log.lines().find_map(|l| l.split_once("terminated by signal ").map(|(_, r)| r)) {
        format!("signal-{}", l.split(':').next().unwrap_or("").trim())
    } else if let Some(l) = log.lines().filter(|l| crate::supervisor::crash_line(l)).find_map(|l| l.split_once("exited with exit code ").map(|(_, r)| r)) {
        format!("exit-{}", l.split_whitespace().next().unwrap_or(""))
    } else {
        "server-lost".into()
    };
    out.truncate(120);
    out
}

/// Write `input` as an example in `dir` (created): the message, the input's
/// Debug, its psql script, and the state it was drawn from.
pub fn write_example(dir: &Path, message: &str, input: &Input, state: &str) {
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let _ = fs::write(dir.join("message.txt"), message);
    let _ = fs::write(dir.join("input.txt"), format!("{:?}\n{:?}\n", input.0, input.1));
    let _ = fs::write(dir.join("script.sql"), script(input, message));
    let _ = fs::write(dir.join("state.txt"), state);
}

/// The identity fields of a finding or an example.
fn provenance(ident: &Identity, worker: usize, n: u64, oracle: &str) -> String {
    format!(
        "\"run\":{},\"campaign\":{},\"work_tag\":{},\"version\":{},\"target\":{},\"worker\":{worker},\"input\":{n},\"oracle\":{}",
        esc(&ident.run),
        esc(&ident.campaign),
        esc(&ident.tag),
        esc(&ident.version),
        esc(&ident.target),
        esc(oracle)
    )
}

/// Count one more under `dir`, writing `finding.json` when it is new.
/// `(new, count)`.
fn count_in(dir: &Path, sig: &str, first: &str, from: &str) -> (bool, u64) {
    let new = !dir.join("finding.json").exists();
    let _ = fs::create_dir_all(dir);
    if new {
        let _ = fs::write(
            dir.join("finding.json"),
            format!("{{\"signature\":{},\"first_line\":{},\"first_seen\":{},{from},\"status\":\"open\"}}\n", esc(sig), esc(first), now()),
        );
    }
    let count_file = dir.join("count");
    let count: u64 = fs::read_to_string(&count_file).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0) + 1;
    let _ = fs::write(&count_file, count.to_string());
    (new, count)
}

fn examples_in(dir: &Path) -> usize {
    fs::read_dir(dir).map_or(0, |d| d.filter_map(Result::ok).filter(|e| e.path().is_dir()).count())
}

/// Record a shrunk failure under its signature: counted always, written while
/// the signature has fewer than [`EXAMPLES`] examples, and an event either
/// way. Whether the signature is new.
#[allow(clippy::too_many_arguments)]
pub fn record_failure(root: &Path, ident: &Identity, worker: usize, n: u64, oracle: &str, message: &str, input: &Input, state: &str) -> bool {
    let sig = signature(oracle, message);
    let dir = root.join("findings").join(&sig);
    let first = message.lines().next().unwrap_or("");
    let from = provenance(ident, worker, n, oracle);
    let (new, count) = count_in(&dir, &sig, first, &from);
    let example = (examples_in(&dir) < EXAMPLES).then(|| dir.join(format!("w{worker}-{n}")));
    if let Some(e) = &example {
        write_example(e, message, input, state);
        let _ = fs::write(e.join("example.json"), format!("{{\"signature\":{},\"ts\":{},{from}}}\n", esc(&sig), now()));
    }
    records::event(
        root,
        &ident.run,
        "finding",
        &format!(
            "\"signature\":{},\"new_signature\":{new},\"count\":{count},\"first_line\":{},\"example\":{},{from}",
            esc(&sig),
            esc(first),
            example.as_ref().map_or("null".into(), |e| esc(&e.to_string_lossy()))
        ),
        &format!("finding {} signature {sig} (worker {worker}, input {n}, {count} seen)", if new { "new" } else { "repeat" }),
    );
    new
}

/// Rebuild the fresh draw a state names (`seed=`, `size=`), when it does.
pub fn rebuild(state: &str) -> Option<Input> {
    let p: std::collections::HashMap<String, String> = pairs(state).into_iter().collect();
    let seed: u64 = p.get("seed")?.parse().ok()?;
    let size: usize = p.get("size")?.parse().ok()?;
    std::panic::catch_unwind(|| case_queries().generate(&mut Source::new(seed, size))).ok()
}

/// Record a server crash: its log excerpt, and each worker's input in flight
/// (`(worker, state)`, rebuilt as a script). `(signature, new, path)`.
pub fn record_crash(root: &Path, log: &str, in_flight: &[(usize, String)], culprits: &[usize]) -> (String, bool, PathBuf) {
    let sig = crash_signature(log);
    let dir = root.join("findings").join("crashes").join(&sig);
    let first = log.lines().find(|l| l.contains("TRAP:") || l.contains("PANIC:") || l.contains("terminated by signal")).unwrap_or("").to_string();
    let from = format!("\"kind\":\"crash\",\"workers_in_flight\":{},\"culprits\":{culprits:?}", in_flight.len());
    let (new, _) = count_in(&dir, &sig, &first, &from);
    let at = dir.join(now().to_string());
    let _ = fs::create_dir_all(&at);
    let mut text = format!(
        "server crash, signature {sig}\nculprit worker(s), whose backend the log names: {culprits:?}\n\n== server log (new since the last check) ==\n{log}\n"
    );
    for (w, state) in in_flight {
        let culprit = if culprits.contains(w) { " (the culprit)" } else { "" };
        text.push_str(&format!("\n== worker {w}{culprit}, input in flight ==\n{state}\n"));
        let _ = fs::write(at.join(format!("w{w}.state.txt")), state);
        if let Some(input) = rebuild(state) {
            let _ = fs::write(at.join(format!("w{w}.sql")), script(&input, &format!("in flight on worker {w} when the server crashed ({sig})\n{state}")));
        }
    }
    if examples_in(&dir) <= EXAMPLES {
        let _ = fs::write(at.join("crash.txt"), text);
    }
    (sig, new, at)
}

/// Record a worker that hung, crashed, went over the memory limit (and so
/// on: `kind` is the folder), with its state file, as `<who>-<time>.txt`
/// (`who` is `w<K>`, or `backend<pid>` for a backend no worker owned).
pub fn record_lost(root: &Path, kind: &str, who: &str, detail: &str, state: &str) -> PathBuf {
    let dir = root.join("findings").join(kind);
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{who}-{}.txt", now()));
    let _ = fs::write(&path, format!("{detail}\n\nstate (the input in flight):\n{state}\n"));
    path
}

/// Failure signatures (directories under `findings/` other than the record
/// folders).
pub fn signatures(root: &Path) -> usize {
    fs::read_dir(root.join("findings")).map_or(0, |d| {
        d.filter_map(Result::ok).filter(|e| e.path().is_dir() && !e.file_name().to_str().is_some_and(|n| RECORD_FOLDERS.contains(&n))).count()
    })
}

/// Crash signatures (directories under `findings/crashes/`).
pub fn crash_signatures(root: &Path) -> usize {
    examples_in(&root.join("findings").join("crashes"))
}

#[cfg(test)]
mod tests {
    use super::{crash_signature, signature};

    #[test]
    fn signatures_fold_numbers_and_sql() {
        assert_eq!(signature("tlp", "tlp-where: results differ\n  original: .."), "tlp-tlp-where__results_differ");
        assert_eq!(signature("model", "statement 12: UPDATE t3 SET c1 = 4: boom 7 (XX000)"), "model-statement-UPDATE-XX000");
        assert_eq!(signature("norec", "the original query fails: SELECT 1\n  x (42883)"), "norec-original-fails-SELECT-42883");
        assert_eq!(signature("model", "model: rows differ\n  statement 3"), "model-model__rows_differ");
    }

    #[test]
    fn crash_signatures_come_from_the_log() {
        let trap = "2026 LOG:  x\nTRAP: failed Assert(\"a > 0\"), File: \"heapam.c\", Line: 123, PID: 456\n.. terminated by signal 6: Abort trap: 6";
        assert_eq!(crash_signature(trap), "trap-failed_Assert__a___N____File___heapam_c___Line__N");
        assert_eq!(crash_signature("LOG:  server process (PID 9) was terminated by signal 11: Segmentation fault"), "signal-11");
        assert_eq!(crash_signature("nothing"), "server-lost");
    }
}
