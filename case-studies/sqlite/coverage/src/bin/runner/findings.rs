// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Findings on disk, under `D/findings/`:
//!
//! - `<signature>/` for failures: the signature is the oracle and the first
//!   line of the failure message with digits folded (`results differ
//!   (tlp-where)`), so one directory gathers one kind of failure, which may
//!   hold more than one bug; up to [`EXAMPLES`] examples each, every one a
//!   directory with `message.txt`, `input.txt` (the shrunk case and queries),
//!   `after.db` (the database after the case's statements, run in memory
//!   whatever the worker's storage, a file the sqlite3 CLI opens; pre-triage
//!   puts a WAL loop's in WAL mode; its main database only, as committed),
//!   `before.db` and `case.sql` (the case's starting database, `case.file`,
//!   and its statements: pre-triage replays a case that leaves a TEMP or
//!   ATTACHed database or an open transaction, which after.db lacks, from
//!   them), `queries.sql` and `example.json` (the
//!   run, version, target, worker, its profile (`gen+wal` for a WAL loop)
//!   and storage (`memory` or `wal`, and for `wal` the settings the case's
//!   WAL file had, `wal_settings`), input and oracle). A statement error's
//!   example (first line `statement N: ...`, or one naming no such column,
//!   table or index: [`keeps_unshrunk`]) also has `unshrunk/`, the input as
//!   it failed before shrinking, with its own message (the same files but
//!   example.json), when shrinking changed it: shrinking keeps a candidate
//!   whose first line is the same, which for such an error can be a
//!   different cause (the statement that added the column removed, say).
//!   Pre-triage reads only the example itself. `count` is how many
//!   were seen in all, and `finding.json` says where the signature was first
//!   seen (run, version, target, worker, profile, storage, time) and holds
//!   its `status`, `open` until triage says otherwise (for instance `fixed
//!   on trunk <check-in>`).
//! - `timeouts/` for inputs that ran past the deadline (the first few of each
//!   worker, unshrunk, laid out the same way).
//! - `crashes/` and `hangs/` for workers that died or stalled, with the input
//!   in flight as their state file gave it.

use std::{
    fs,
    path::{Path, PathBuf},
};

use ephyra_sqlite::sqlancer::{self, Storage};

use crate::{
    now,
    records::{self, esc, Identity},
    Input,
};

/// Examples kept per signature.
pub const EXAMPLES: usize = 20;

/// The signature of a failure: the oracle and the message's first line,
/// digits folded to `N`, made safe as a directory name.
pub fn signature(oracle: &str, message: &str) -> String {
    let first = message.lines().next().unwrap_or("");
    let mut out = String::from(oracle);
    out.push('-');
    let mut last_digit = false;
    for ch in first.chars() {
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
    out.truncate(120);
    out
}

/// Whether a failure is a statement error, whose example keeps the input it
/// failed with before shrinking (`unshrunk/`): its first line is a case's
/// statement failing (`statement N: <sql>: <error>`, `sqlancer`'s
/// `run_statement`), or names no such column, table or index.
pub fn keeps_unshrunk(message: &str) -> bool {
    let first = message.lines().next().unwrap_or("");
    let statement = first.strip_prefix("statement ").is_some_and(|r| {
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        digits > 0 && r[digits..].starts_with(':')
    });
    statement || ["no such column", "no such table", "no such index"].iter().any(|e| first.contains(e))
}

/// The directory of an example that holds its input before shrinking.
pub const UNSHRUNK: &str = "unshrunk";

/// Write `input` as an example in `dir` (created): the message, the input,
/// the database after its statements (in memory), the case's starting
/// database and its statements, and its queries.
pub fn write_example(dir: &Path, message: &str, input: &Input) {
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let _ = fs::write(dir.join("message.txt"), message);
    let _ = fs::write(dir.join("input.txt"), format!("{:?}\n{:?}\n", input.0, input.1));
    let queries: String = input.1.iter().map(|q| format!("{};\n", q.sql())).collect();
    let _ = fs::write(dir.join("queries.sql"), queries);
    let _ = fs::write(dir.join("before.db"), &input.0.file.0);
    let case: String = input.0.statements.iter().map(|s| format!("{s};\n")).collect();
    let _ = fs::write(dir.join("case.sql"), case);
    if let Ok(db) = sqlancer::open_with(&input.0, &Storage::Memory) {
        if let Ok(bytes) = db.serialize(rusqlite::MAIN_DB) {
            let _ = fs::write(dir.join("after.db"), &*bytes);
        }
    }
}

/// The identity fields of a finding or an example.
fn provenance(ident: &Identity, worker: usize, profile: &str, storage: &str, n: u64, oracle: &str) -> String {
    format!(
        "\"run\":{},\"campaign\":{},\"work_tag\":{},\"version\":{},\"target\":{},\"worker\":{worker},\"profile\":{},\"storage\":{},\"input\":{n},\"oracle\":{}",
        esc(&ident.run),
        esc(&ident.campaign),
        esc(&ident.tag),
        esc(&ident.version),
        esc(&ident.target),
        esc(profile),
        esc(storage),
        esc(oracle)
    )
}

/// For an example of a WAL loop, the settings its case's WAL file had
/// (`sqlancer::wal_settings`) as a JSON field after a comma; else nothing.
fn wal_settings(storage: &str, input: &Input) -> String {
    if storage != "wal" {
        return String::new();
    }
    let w = sqlancer::wal_settings(&input.0);
    format!(
        ",\"wal_settings\":{{\"cache_size\":{},\"autocheckpoint\":{},\"synchronous\":{},\"reopen\":{}}}",
        w.cache_size,
        w.autocheckpoint,
        esc(w.synchronous),
        w.reopen
    )
}

/// Record a shrunk failure under its signature: counted always, written while
/// the signature has fewer than [`EXAMPLES`] examples, and an event either
/// way. `original` is the failure before shrinking (its message and input):
/// a statement error's example ([`keeps_unshrunk`]) writes it under
/// [`UNSHRUNK`] when it differs from `input`. Whether the signature is new.
#[allow(clippy::too_many_arguments)]
pub fn record_failure(root: &Path, ident: &Identity, worker: usize, profile: &str, storage: &str, n: u64, oracle: &str, message: &str, input: &Input, original: Option<(&str, &Input)>) -> bool {
    let sig = signature(oracle, message);
    let dir = root.join("findings").join(&sig);
    let new = !dir.exists();
    let _ = fs::create_dir_all(&dir);
    let first = message.lines().next().unwrap_or("");
    let from = provenance(ident, worker, profile, storage, n, oracle);
    if new {
        let _ = fs::write(
            dir.join("finding.json"),
            format!("{{\"signature\":{},\"first_line\":{},\"first_seen\":{},{from},\"status\":\"open\"}}\n", esc(&sig), esc(first), now()),
        );
    }
    let count_file = dir.join("count");
    let count: u64 = fs::read_to_string(&count_file).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0) + 1;
    let _ = fs::write(&count_file, count.to_string());
    let examples = fs::read_dir(&dir).map_or(0, |d| d.filter_map(Result::ok).filter(|e| e.path().is_dir()).count());
    let example = (examples < EXAMPLES).then(|| dir.join(format!("w{worker}-{n}")));
    let mut unshrunk = false;
    if let Some(e) = &example {
        write_example(e, message, input);
        if let Some((why, before)) = original.filter(|_| keeps_unshrunk(message)) {
            if format!("{before:?}") != format!("{input:?}") {
                write_example(&e.join(UNSHRUNK), why, before);
                unshrunk = true;
            }
        }
        let _ = fs::write(e.join("example.json"), format!("{{\"signature\":{},\"ts\":{},{from}{}}}\n", esc(&sig), now(), wal_settings(storage, input)));
    }
    records::event(
        root,
        &ident.run,
        "finding",
        &format!(
            "\"signature\":{},\"new_signature\":{new},\"count\":{count},\"first_line\":{},\"example\":{},\"unshrunk\":{unshrunk},{from}",
            esc(&sig),
            esc(first),
            example.as_ref().map_or("null".into(), |e| esc(&e.to_string_lossy()))
        ),
        &format!("finding {} signature {sig} (worker {worker}, {profile}, input {n}, {count} seen)", if new { "new" } else { "repeat" }),
    );
    new
}

/// Record an input that ran past the deadline, unshrunk, with an event.
#[allow(clippy::too_many_arguments)]
pub fn record_timeout(root: &Path, ident: &Identity, worker: usize, profile: &str, storage: &str, n: u64, oracle: &str, input: &Input) {
    let dir = root.join("findings").join("timeouts").join(format!("w{worker}-{n}"));
    write_example(&dir, &format!("timeout under {oracle} ({storage})"), input);
    let from = provenance(ident, worker, profile, storage, n, oracle);
    let _ = fs::write(dir.join("example.json"), format!("{{\"ts\":{},{from}{}}}\n", now(), wal_settings(storage, input)));
    records::event(root, &ident.run, "timeout_saved", &format!("\"example\":{},{from}", esc(&dir.to_string_lossy())), &format!("timeout saved (worker {worker}, {profile}, input {n}, {oracle})"));
}

/// Record a worker that crashed or hung (`kind`), with its state file.
pub fn record_lost(root: &Path, kind: &str, worker: usize, detail: &str, state: &str) -> PathBuf {
    let dir = root.join("findings").join(kind);
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("w{worker}-{}.txt", crate::now()));
    let _ = fs::write(&path, format!("{detail}\n\nstate (the input in flight):\n{state}\n"));
    path
}

/// Signatures (directories under `findings/` other than the timeout, crash
/// and hang ones).
pub fn signatures(root: &Path) -> usize {
    fs::read_dir(root.join("findings")).map_or(0, |d| {
        d.filter_map(Result::ok)
            .filter(|e| e.path().is_dir() && !matches!(e.file_name().to_str(), Some("timeouts" | "crashes" | "hangs")))
            .count()
    })
}

#[cfg(test)]
mod tests {
    use super::{keeps_unshrunk, signature};

    #[test]
    fn statement_errors_keep_the_unshrunk_input() {
        for m in [
            "statement 8: UPDATE t1 SET c5 = 5 WHERE c1 BETWEEN NULL AND -56712.0: no such column: c5",
            "statement 0: CREATE TABLE t0(c0): table t0 already exists\n  more",
            "the original query fails: SELECT * FROM aux.t6: no such table: aux.t6",
            "results differ (plans-left): no such index: i0",
        ] {
            assert!(keeps_unshrunk(m), "{m}");
        }
        for m in ["results differ (tlp-where)\n  statement 3: x: no such column: c0", "statement x: y", "statements 3: y", "statement 3 y", ""] {
            assert!(!keeps_unshrunk(m), "{m}");
        }
    }

    #[test]
    fn signatures_fold_digits_and_odd_characters() {
        assert_eq!(signature("tlp", "results differ (tlp-where)\n  original: .."), "tlp-results_differ__tlp-where_");
        assert_eq!(signature("norec", "statement 12: x"), "norec-statement_N__x");
    }
}
