// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Differential testing of Turso against SQLite 3.50.4, the version Turso
//! tracks (COMPAT.md: any undocumented deviation from it is a bug), on the
//! cases and queries of ephyra-sqlite's generator.
//!
//! Each input is a case (a database file and statements) and its queries.
//! Both engines open the same file: SQLite deserialized in memory, as the
//! oracles do, Turso from a copy on disk. Both run the case's statements,
//! then each query, and the results are compared:
//!
//! - rows in order when the query's ORDER BY fixes the order
//!   ([`Query::total_order`]), else as multisets;
//! - text as UTF-8 with U+FFFD for invalid bytes, Turso's documented
//!   behavior (COMPAT.md, "Text values must be valid UTF-8");
//! - a BLOB that is valid JSONB by its JSON text (`json(x)`), since one value
//!   has more than one JSONB encoding.
//!
//! What is not a difference is counted, by reason: a case SQLite rejects,
//! and a statement or query Turso rejects as unsupported. The generator
//! follows SQLite trunk: its statements newer than 3.50.4 are dropped before
//! anything runs ([`newer_than_3_50_4`]), and SQLite is built with the
//! extensions the generator draws (`.cargo/config.toml`: Geopoly and STAT4,
//! checked at startup, [`sqlite_features`]). When a statement is dropped,
//! the rest is settled against the generator's model ([`settle`]). A Turso
//! panic, a query only one engine fails, rows that differ, and a statement
//! only Turso fails for another reason are findings, kept per signature
//! (at most [`EXAMPLES`] each) and shrunk with the generator's shrinker.
//! A worker stuck on one input for [`HANG`] is a hang finding.
//!
//! Before a query finding is kept, SQLite trunk runs the same input
//! ([`trunk_check`]): when trunk gives what Turso gave, the difference is a
//! SQLite 3.50.4 behavior newer SQLite changed, and it is skipped. Trunk
//! runs in a helper program, `trunk-oracle/` (two SQLite versions can't be
//! linked into one), which must be built first; the runner checks it at
//! startup and exits if it is missing or broken:
//!
//! ```sh
//! cd trunk-oracle && env -u CARGO_TARGET_DIR cargo build --release
//! ```
//!
//! It is looked for at `trunk-oracle/target/release/trunk-oracle` (a
//! CARGO_TARGET_DIR would put it elsewhere), or where
//! `TURSO_DIFF_TRUNK_ORACLE` says. `--no-trunk-oracle` runs without it.
//!
//! `cargo run --release -- --seed S --minutes M --jobs J --out DIR [--experimental] [--no-trunk-oracle]`
//! (from this directory, for `.cargo/config.toml`'s SQLite flags)
//!
//! `turso-diff --replay DB SQL` runs a script on both engines;
//! `turso-diff --retriage RUN_DIR` classifies a run's kept examples again
//! with the current rules ([`retriage`]).

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    ops::ControlFlow,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use ephyra::{Gen, Source};
use ephyra_sqlite::{
    cases::{self, Case},
    SqliteFile,
    expr::{Query, Schema},
    probes::{case_queries, final_model, final_schema, repair_queries},
    semantics,
    statements::Statement,
};
use rusqlite::{types::ValueRef, Connection};

type Input = (Case, Vec<Query>);

/// Examples kept per signature.
const EXAMPLES: usize = 3;
/// A worker on one input this long is hung.
const HANG: Duration = Duration::from_secs(60);
/// SQLite's budget per statement or query.
const SQLITE_BUDGET: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
    Json(String),
}

type Rows = Vec<Vec<Cell>>;

/// What one input gave.
#[derive(Default)]
struct Outcome {
    queries: u64,
    compared: u64,
    skips: Vec<String>,
    findings: Vec<Finding>,
    /// The statement that ended the input (SQLite or Turso rejected it):
    /// the runner drops it and runs the rest again ([`RETRIES`]).
    failed_statement: Option<usize>,
    /// Findings SQLite trunk was asked about, and why the helper failed on
    /// some ([`trunk_check`]).
    trunk_checks: u64,
    trunk_failures: Vec<String>,
}

#[derive(Clone, Default)]
struct Finding {
    /// The kind and a normalized message: what examples are grouped by.
    signature: String,
    /// What to show: the statement or query, and what each engine gave.
    detail: String,
    /// The error or panic message as it is (empty for differing rows): a
    /// shrunk input must give the same one, not just the same signature.
    exact: String,
    /// The index of the query it came from (none for the file or a
    /// statement): the queries before it are part of the reproduction,
    /// since they can change the connection's state.
    query: Option<usize>,
    /// What Turso gave for that query, and whether rows were compared in
    /// order: SQLite trunk's result is compared with it ([`trunk_agrees`]).
    turso: Option<Result<Rows, String>>,
    ordered: bool,
}

// ---------------------------------------------------------------- SQLite

thread_local! {
    /// What Turso is running, for a panic's finding.
    static STEP: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static DEADLINE: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
    /// Where the last panic on this thread was raised (the panic hook keeps
    /// the default message off stderr): for [`panic_message`].
    static PANIC_AT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn past_deadline() -> bool {
    DEADLINE.with(|d| d.get().is_some_and(|t| Instant::now() > t))
}

fn sqlite_budget() {
    DEADLINE.with(|d| d.set(Some(Instant::now() + SQLITE_BUDGET)));
}

/// An empty in-memory SQLite, foreign keys off as Turso starts (a case's
/// own PRAGMA foreign_keys still runs in both engines; the generator's
/// trunk build defines SQLITE_DEFAULT_FOREIGN_KEYS=1, this one does not).
fn sqlite_memory() -> Result<Connection, String> {
    let db = Connection::open_in_memory().map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA foreign_keys = OFF").map_err(|e| e.to_string())?;
    Ok(db)
}

/// The case's file in memory, its statements run; or the first error.
fn open_sqlite(case: &Case) -> Result<Connection, String> {
    let mut db = sqlite_memory()?;
    db.progress_handler(10_000, Some(past_deadline));
    let bytes = &case.file.0;
    if !bytes.is_empty() {
        // In memory there is no WAL: a WAL header is read as rollback mode.
        let mut bytes = bytes.to_vec();
        for at in [18, 19] {
            if bytes[at] == 2 {
                bytes[at] = 1;
            }
        }
        db.deserialize_read_exact(rusqlite::MAIN_DB, &bytes[..], bytes.len(), false).map_err(|e| e.to_string())?;
    }
    for (k, s) in case.statements.iter().enumerate() {
        sqlite_budget();
        db.execute_batch(&s.to_string()).map_err(|e| format!("statement {k}: {e}"))?;
    }
    Ok(db)
}

fn sqlite_cell(v: ValueRef) -> Cell {
    match v {
        ValueRef::Null => Cell::Null,
        ValueRef::Integer(i) => Cell::Int(i),
        ValueRef::Real(x) => Cell::Real(x),
        ValueRef::Text(t) => Cell::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Cell::Blob(b.to_vec()),
    }
}

fn sqlite_query(db: &Connection, sql: &str) -> Result<Rows, String> {
    sqlite_budget();
    let go = || -> rusqlite::Result<Rows> {
        let mut stmt = db.prepare(sql)?;
        let n = stmt.column_count();
        let rows = stmt.query_map([], |r| (0..n).map(|k| r.get_ref(k).map(sqlite_cell)).collect::<Result<Vec<_>, _>>())?;
        rows.collect()
    };
    go().map_err(|e| e.to_string())
}

/// The query's result in SQLite without `sqlite_stat4`, or None when SQLite
/// has no `sqlite_stat4`. This build has SQLITE_ENABLE_STAT4
/// (`.cargo/config.toml`), so its ANALYZE creates `sqlite_stat4` next to
/// `sqlite_stat1`; Turso's ANALYZE creates `sqlite_stat1` only, and Turso
/// rejects reads and writes of `sqlite_stat4` as unsupported. Listings of
/// the schema (pragma_table_list, sqlite_schema, ...) then differ by that
/// table alone. The table is dropped inside a savepoint that is rolled
/// back, so the statements and the other queries still see it (the
/// generator's crafted statistics need it); only a query that already
/// differs is run again.
fn sqlite_without_stat4(db: &Connection, sql: &str) -> Option<Result<Rows, String>> {
    let has = db.query_row("SELECT count(*) FROM sqlite_schema WHERE name = 'sqlite_stat4'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if !has || db.execute_batch("SAVEPOINT turso_diff_stat4").is_err() {
        return None;
    }
    let rows = db.execute_batch("DROP TABLE sqlite_stat4").ok().map(|()| sqlite_query(db, sql));
    let _ = db.execute_batch("ROLLBACK TO turso_diff_stat4; RELEASE turso_diff_stat4;");
    rows
}

/// Every BLOB that is valid JSONB as its JSON text, by SQLite.
fn jsonb_as_json(db: &Connection, rows: &mut Rows) {
    let Ok(mut text) = db.prepare_cached("SELECT json(?1) WHERE json_valid(?1, 8)") else { return };
    for c in rows.iter_mut().flatten() {
        if let Cell::Blob(b) = c {
            if let Ok(Some(t)) = text.query_row([&b[..]], |r| r.get::<_, Option<String>>(0)) {
                *c = Cell::Json(t);
            }
        }
    }
}

/// The case as both engines get it. Turso writes an auto-vacuum database
/// only with `--experimental-autovacuum`, which its Rust API cannot set
/// (it opens such a file read-only, COMPAT.md): SQLite rewrites the file
/// without auto-vacuum first (`PRAGMA auto_vacuum = 0; VACUUM`), and both
/// engines open the rewritten file.
fn prepared(case: &Case) -> Result<Case, String> {
    let c = without_autovacuum(case)?;
    let c = if utf16(&c.file.0) { Case { file: SqliteFile(to_utf8(&c.file.0)?), statements: c.statements } } else { c };
    drop_without_rowid_indexes(&c)
}

fn utf16(bytes: &[u8]) -> bool {
    bytes.len() >= 60 && bytes[56..60] != [0, 0, 0, 1] && bytes[56..60] != [0, 0, 0, 0]
}

/// The file in memory, a WAL header read as rollback mode.
fn deserialized(bytes: &[u8]) -> Result<Connection, String> {
    let mut db = sqlite_memory()?;
    let mut b = bytes.to_vec();
    for at in [18, 19] {
        if b[at] == 2 {
            b[at] = 1;
        }
    }
    db.deserialize_read_exact(rusqlite::MAIN_DB, &b[..], b.len(), false).map_err(|e| e.to_string())?;
    Ok(db)
}

/// A UTF-8 copy of a UTF-16 database: Turso reads UTF-8 only (COMPAT.md),
/// so its content is copied, as SQLite reads it, into a new file: the page
/// size, user version and application id, the tables with their rows (and
/// rowids), `sqlite_sequence` and `sqlite_stat1`, then the indexes, views
/// and triggers. A virtual table, or anything that fails, gives an error
/// (the input is skipped, as before).
fn to_utf8(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let e = |e: rusqlite::Error| format!("utf-16 to utf-8 copy: {e}");
    let src = deserialized(bytes)?;
    let dst = sqlite_memory().map_err(|m| format!("utf-16 to utf-8 copy: {m}"))?;
    let page: i64 = src.query_row("PRAGMA page_size", [], |r| r.get(0)).map_err(e)?;
    let uv: i64 = src.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(e)?;
    let app: i64 = src.query_row("PRAGMA application_id", [], |r| r.get(0)).map_err(e)?;
    dst.execute_batch(&format!("PRAGMA page_size = {page}; PRAGMA user_version = {uv}; PRAGMA application_id = {app};")).map_err(e)?;
    let objects: Vec<(String, String, String)> = {
        let mut st = src.prepare("SELECT type, name, sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY rowid").map_err(e)?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(e)?;
        rows.collect::<Result<_, _>>().map_err(e)?
    };
    if objects.iter().any(|(_, _, sql)| sql.to_uppercase().starts_with("CREATE VIRTUAL")) {
        return Err("utf-16 to utf-8 copy: a virtual table".into());
    }
    let q = |n: &str| format!("\"{}\"", n.replace('"', "\"\""));
    for (ty, name, sql) in &objects {
        if ty != "table" {
            continue;
        }
        dst.execute_batch(sql).map_err(e)?;
        let cols: Vec<String> = {
            let mut st = src.prepare(&format!("SELECT name FROM pragma_table_xinfo({}) WHERE hidden = 0", q(name).replace('"', "'"))).map_err(e)?;
            let rows = st.query_map([], |r| r.get::<_, String>(0)).map_err(e)?;
            rows.collect::<Result<_, _>>().map_err(e)?
        };
        let rowid = !sql.to_uppercase().contains("WITHOUT ROWID");
        let list = cols.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ");
        let n = cols.len() + usize::from(rowid);
        let select = if rowid { format!("SELECT rowid, {list} FROM {}", q(name)) } else { format!("SELECT {list} FROM {}", q(name)) };
        let marks = (1..=n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
        let try_insert = |with_rowid: bool| -> rusqlite::Result<()> {
            let target = if with_rowid { format!("rowid, {list}") } else { list.clone() };
            let skip = usize::from(rowid && !with_rowid);
            let m = (1..=n - skip).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
            let mut ins = dst.prepare(&format!("INSERT INTO {}({target}) VALUES ({})", q(name), if skip == 0 { marks.clone() } else { m }))?;
            let mut st = src.prepare(&select)?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                let vals: Vec<rusqlite::types::Value> = (skip..n).map(|k| r.get::<_, rusqlite::types::Value>(k)).collect::<Result<_, _>>()?;
                ins.execute(rusqlite::params_from_iter(vals))?;
            }
            Ok(())
        };
        if try_insert(rowid).is_err() {
            dst.execute_batch(&format!("DELETE FROM {}", q(name))).map_err(e)?;
            try_insert(false).map_err(e)?;
        }
    }
    let has = |t: &str| src.query_row("SELECT count(*) FROM sqlite_schema WHERE name = ?1", [t], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if has("sqlite_sequence") {
        dst.execute_batch("DELETE FROM sqlite_sequence").map_err(e)?;
        let rows: Vec<(String, i64)> = {
            let mut st = src.prepare("SELECT name, seq FROM sqlite_sequence").map_err(e)?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?;
            rows.collect::<Result<_, _>>().map_err(e)?
        };
        for (n, v) in rows {
            dst.execute("INSERT INTO sqlite_sequence(name, seq) VALUES (?1, ?2)", rusqlite::params![n, v]).map_err(e)?;
        }
    }
    if has("sqlite_stat1") {
        dst.execute_batch("ANALYZE sqlite_schema; DELETE FROM sqlite_stat1;").map_err(e)?;
        // sqlite_stat4 samples are records in the file's encoding: not copied.
        let _ = dst.execute_batch("DELETE FROM sqlite_stat4;");
        let rows: Vec<(String, Option<String>, Option<String>)> = {
            let mut st = src.prepare("SELECT tbl, idx, stat FROM sqlite_stat1").map_err(e)?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(e)?;
            rows.collect::<Result<_, _>>().map_err(e)?
        };
        for (t, i, st) in rows {
            dst.execute("INSERT INTO sqlite_stat1(tbl, idx, stat) VALUES (?1, ?2, ?3)", rusqlite::params![t, i, st]).map_err(e)?;
        }
    }
    for (ty, _, sql) in &objects {
        if ty != "table" {
            dst.execute_batch(sql).map_err(e)?;
        }
    }
    let out = dst.serialize(rusqlite::MAIN_DB).map_err(e)?;
    Ok(out.to_vec())
}

/// Names of the WITHOUT ROWID tables in a database.
fn without_rowid_tables(db: &Connection) -> Vec<String> {
    let Ok(mut st) = db.prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND upper(sql) LIKE '%WITHOUT ROWID%'") else { return Vec::new() };
    st.query_map([], |r| r.get::<_, String>(0)).map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
}

/// The case with every index on a WITHOUT ROWID table dropped from its
/// file: Turso reads such an index's other columns as NULL (turso #7273,
/// known), and SQLite cannot create one in Turso anyway. A statement that
/// names a dropped index is settled away before the input runs ([`model_base`]).
fn drop_without_rowid_indexes(case: &Case) -> Result<Case, String> {
    let bytes = &case.file.0;
    if bytes.len() < 100 {
        return Ok(case.clone());
    }
    let db = deserialized(bytes)?;
    let idx: Vec<String> = {
        let Ok(mut st) = db.prepare(
            "SELECT i.name FROM sqlite_schema i JOIN sqlite_schema t ON i.tbl_name = t.name \
             WHERE i.type = 'index' AND i.sql IS NOT NULL AND t.type = 'table' AND upper(t.sql) LIKE '%WITHOUT ROWID%'",
        ) else {
            return Ok(case.clone());
        };
        st.query_map([], |r| r.get::<_, String>(0)).map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    };
    if idx.is_empty() {
        return Ok(case.clone());
    }
    for i in &idx {
        db.execute_batch(&format!("DROP INDEX \"{}\"", i.replace('"', "\"\""))).map_err(|e| format!("dropping {i}: {e}"))?;
    }
    let out = db.serialize(rusqlite::MAIN_DB).map_err(|e| e.to_string())?;
    Ok(Case { file: SqliteFile(out.to_vec()), statements: case.statements.clone() })
}

/// A note for findings on a WITHOUT ROWID table whose primary key columns
/// are not its leading columns: Turso's INSERT applies the affinities in
/// schema order there (turso #6749).
fn affinity_order_note(db: &Connection) -> String {
    for t in without_rowid_tables(db) {
        let Ok(mut st) = db.prepare("SELECT cid, pk FROM pragma_table_info(?1) ORDER BY cid") else { continue };
        let cols: Vec<(i64, i64)> = st.query_map([&t], |r| Ok((r.get(0)?, r.get(1)?))).map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default();
        let mut pk: Vec<(i64, i64)> = cols.iter().filter(|c| c.1 > 0).map(|c| (c.1, c.0)).collect();
        pk.sort();
        if pk.iter().enumerate().any(|(i, (_, cid))| *cid != i as i64) {
            return "note: WITHOUT ROWID table with non-leading primary key columns (turso #6749 shape)\n".into();
        }
    }
    String::new()
}

/// The case with auto-vacuum turned off (see [`prepared`]).
fn without_autovacuum(case: &Case) -> Result<Case, String> {
    let bytes = &case.file.0;
    if bytes.len() < 100 || bytes[52..56] == [0, 0, 0, 0] {
        return Ok(case.clone());
    }
    let mut db = sqlite_memory()?;
    let mut b = bytes.to_vec();
    for at in [18, 19] {
        if b[at] == 2 {
            b[at] = 1;
        }
    }
    db.deserialize_read_exact(rusqlite::MAIN_DB, &b[..], b.len(), false).map_err(|e| e.to_string())?;
    sqlite_budget();
    db.execute_batch("PRAGMA auto_vacuum = 0; VACUUM;").map_err(|e| format!("rewriting without auto-vacuum: {e}"))?;
    let out = db.serialize(rusqlite::MAIN_DB).map_err(|e| e.to_string())?;
    Ok(Case { file: SqliteFile(out.to_vec()), statements: case.statements.clone() })
}

// ----------------------------------------------------------------- Turso

struct Turso {
    _db: turso::Database,
    conn: turso::Connection,
}

/// A fresh copy of the case's file at `path`, opened by Turso.
fn open_turso(case: &Case, path: &Path, experimental: bool) -> Result<Turso, String> {
    for ext in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{ext}", path.display()));
    }
    fs::write(path, &case.file.0).map_err(|e| format!("writing {}: {e}", path.display()))?;
    let mut b = turso::Builder::new_local(&path.to_string_lossy());
    if experimental {
        b = b.experimental_without_rowid(true).experimental_generated_columns(true).experimental_attach(true);
    }
    STEP.with(|s| *s.borrow_mut() = "(opening the file)".to_string());
    let db = pollster::block_on(b.build()).map_err(|e| format!("open: {e}"))?;
    let conn = db.connect().map_err(|e| format!("connect: {e}"))?;
    Ok(Turso { _db: db, conn })
}

fn turso_cell(v: turso::Value) -> Cell {
    match v {
        turso::Value::Null => Cell::Null,
        turso::Value::Integer(i) => Cell::Int(i),
        turso::Value::Real(x) => Cell::Real(x),
        turso::Value::Text(t) => Cell::Text(t),
        turso::Value::Blob(b) => Cell::Blob(b),
    }
}

fn turso_query(t: &Turso, sql: &str) -> Result<Rows, String> {
    STEP.with(|s| *s.borrow_mut() = sql.to_string());
    pollster::block_on(async {
        let mut rows = t.conn.query(sql, ()).await.map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
            let n = row.column_count();
            out.push((0..n).map(|k| row.get_value(k).map(turso_cell)).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

/// `sql` split into single statements, lexically, by SQLite's
/// `sqlite3_complete()` (a `;` that ends a complete statement).
fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in sql.chars() {
        cur.push(c);
        if c == ';' {
            let z = std::ffi::CString::new(cur.as_str()).unwrap_or_default();
            // SAFETY: a NUL-terminated string that outlives the call.
            if unsafe { rusqlite::ffi::sqlite3_complete(z.as_ptr()) } != 0 {
                out.push(std::mem::take(&mut cur));
            }
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// The entries of a script [`script`] wrote (one per line, `;`-terminated;
/// one entry can hold several statements, or a literal with a newline):
/// lines joined until they form complete SQL.
fn script_entries(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.split_inclusive('\n') {
        cur.push_str(line);
        let z = std::ffi::CString::new(cur.as_str()).unwrap_or_default();
        // SAFETY: a NUL-terminated string that outlives the call.
        if unsafe { rusqlite::ffi::sqlite3_complete(z.as_ptr()) } != 0 {
            let s = std::mem::take(&mut cur);
            let s = s.trim_end().trim_end_matches(';');
            if !s.trim().is_empty() {
                out.push(s.to_string());
            }
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim_end().to_string());
    }
    out
}

/// Each statement of `sql` in turn, its rows (a PRAGMA's report) read and
/// dropped: `execute_batch` refuses a statement that returns rows.
fn turso_exec(t: &Turso, sql: &str) -> Result<(), String> {
    for one in split_statements(sql) {
        turso_query(t, &one)?;
    }
    Ok(())
}

// ------------------------------------------------------------ comparison

/// A Turso error that names a missing feature rather than a wrong answer.
/// `sqlite_stat4` exists only in a SQLite built with SQLITE_ENABLE_STAT4,
/// as this one is (`.cargo/config.toml`) and Turso's default-build target
/// is not.
fn unsupported(e: &str) -> bool {
    let l = e.to_lowercase();
    if l.contains("no such table: sqlite_stat4") {
        return true;
    }
    // dbstat exists only with SQLITE_ENABLE_DBSTAT_VTAB (this build, not a
    // default one); COMPAT.md lists PRAGMA collation_list and
    // compile_options as unsupported and locking_mode as EXCLUSIVE only.
    if ["no such table: dbstat", "no such table: main.dbstat", "no such table: temp.dbstat", "no such table: pragma_compile_options", "no such table: pragma_collation_list", "locking_mode must be exclusive"].iter().any(|p| l.contains(p)) {
        return true;
    }
    // SQLite's FTS3/4/5 (documented: Turso has its own FTS) and R-Tree,
    // which Turso doesn't ship.
    if ["fts3", "fts4", "fts5", "rtree", "rtree_i32", "geopoly"].iter().any(|m| l.contains(&format!("no such module: {m}"))) {
        return true;
    }
    ["not supported", "unsupported", "not yet supported", "not implemented", "not yet implemented", "is disabled", "experimental"]
        .iter()
        .any(|p| l.contains(p))
}

/// A message with numbers, quoted strings and names collapsed, for grouping.
fn normalize(e: &str) -> String {
    let mut out = String::new();
    let mut chars = e.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                chars.next();
            }
            out.push('N');
        } else if c == '\'' || c == '"' || c == '`' {
            for d in chars.by_ref() {
                if d == c {
                    break;
                }
            }
            out.push_str("'?'");
        } else {
            out.push(c);
        }
    }
    let out: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    out.chars().take(90).collect()
}

fn sort_key(c: &Cell) -> (u8, String) {
    match c {
        Cell::Null => (0, String::new()),
        Cell::Int(i) => (1, format!("{:020}", *i as i128 + (1i128 << 63))),
        Cell::Real(x) => (2, format!("{x:e}")),
        Cell::Text(t) => (3, t.clone()),
        Cell::Blob(b) => (4, format!("{b:02x?}")),
        Cell::Json(j) => (5, j.clone()),
    }
}

fn sorted(rows: &Rows) -> Rows {
    let mut r = rows.clone();
    r.sort_by_key(|row| row.iter().map(sort_key).collect::<Vec<_>>());
    r
}

fn real_close(a: &Rows, b: &Rows) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.len() == y.len()
                && x.iter().zip(y).all(|(p, q)| match (p, q) {
                    (Cell::Real(u), Cell::Real(v)) => (u - v).abs() <= 1e-12 * u.abs().max(v.abs()).max(1.0),
                    _ => p == q,
                })
        })
}

fn show(rows: &Result<Rows, String>) -> String {
    match rows {
        Err(e) => format!("error: {e}"),
        Ok(rows) => {
            let mut s = format!("{} rows", rows.len());
            for r in rows.iter().take(20) {
                let cells: Vec<String> = r
                    .iter()
                    .map(|c| match c {
                        Cell::Null => "NULL".into(),
                        Cell::Int(i) => i.to_string(),
                        Cell::Real(x) => format!("{x:?}"),
                        Cell::Text(t) => format!("{t:?}"),
                        Cell::Blob(b) => format!("x'{}'", b.iter().map(|x| format!("{x:02x}")).collect::<String>()),
                        Cell::Json(j) => format!("jsonb {j}"),
                    })
                    .collect();
                let _ = write!(s, "\n  {}", cells.join("|"));
            }
            if rows.len() > 20 {
                s.push_str("\n  ...");
            }
            s
        },
    }
}

/// Aggregates whose value depends on the order of their input rows.
fn order_dependent(sql: &str) -> bool {
    let l = sql.to_lowercase();
    ["group_concat(", "string_agg(", "json_group_array(", "json_group_object(", "jsonb_group_array(", "jsonb_group_object("].iter().any(|f| l.contains(f))
}

/// A finding when SQLite and Turso disagree on whether a transaction is
/// open after `what` ran: Turso ends a transaction on any runtime error
/// (turso #6864), and every later result then differs for that reason.
fn transaction_state(sdb: &Connection, t: &Turso, what: &str, query: Option<usize>) -> Option<Finding> {
    let s = sdb.is_autocommit();
    let r = t.conn.is_autocommit().unwrap_or(s);
    (s != r).then(|| {
        let state = |auto: bool| if auto { "no transaction" } else { "in a transaction" };
        Finding {
            signature: format!("transaction-state: SQLite {}, Turso {} (turso #6864?)", state(s), state(r)),
            detail: format!("after {what}: SQLite {}, Turso {}", state(s), state(r)),
            exact: String::new(),
            query,
            ..Finding::default()
        }
    })
}

/// Rows with each whole-number REAL as its integer, and (outside JSON) -0.0
/// as 0: SQLite stores a whole-number REAL as an integer on disk and makes
/// it a REAL again when it reads it; DISTINCT, GROUP BY and compounds keep
/// either of two equal values.
fn canonical(rows: &Rows) -> Rows {
    rows.iter()
        .map(|r| {
            r.iter()
                .map(|c| match c {
                    Cell::Real(x) if x.fract() == 0.0 && x.abs() < 9.007_199_254_740_992e15 => Cell::Int(*x as i64),
                    c => c.clone(),
                })
                .collect()
        })
        .collect()
}

/// `rows` with each BLOB cell made TEXT (same bytes) where `like` has TEXT
/// in that place: Turso's `x || y` of blobs is a blob, SQLite's is text
/// (turso #6089).
fn text_for_blobs(rows: &Rows, like: &Rows) -> Rows {
    rows.iter()
        .zip(like.iter().chain(std::iter::repeat(&Vec::new())))
        .map(|(r, l)| {
            r.iter()
                .enumerate()
                .map(|(i, c)| match (c, l.get(i)) {
                    (Cell::Blob(b), Some(Cell::Text(_))) => Cell::Text(String::from_utf8_lossy(b).into_owned()),
                    (c, _) => c.clone(),
                })
                .collect()
        })
        .collect()
}

fn has_fffd(rows: &Rows) -> bool {
    rows.iter().flatten().any(|c| match c {
        Cell::Text(t) | Cell::Json(t) => t.contains('\u{FFFD}') || t.to_lowercase().contains("efbfbd") || t.contains("65533"),
        Cell::Blob(b) => b.windows(3).any(|w| w == [0xEF, 0xBF, 0xBD]),
        _ => false,
    })
}

fn negative_zero_cells(rows: &Rows) -> bool {
    rows.iter().flatten().any(|c| matches!(c, Cell::Real(x) if *x == 0.0 && x.is_sign_negative()))
}

/// The trunk-oracle helper (SQLite trunk, a separate program: two SQLite
/// versions can't be linked into one), where `cargo build --release` in
/// `trunk-oracle/` puts it ([`TRUNK_ORACLE_BUILD`]);
/// `TURSO_DIFF_TRUNK_ORACLE` names another path.
const TRUNK_ORACLE_DEFAULT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/trunk-oracle/target/release/trunk-oracle");
/// The command that builds the helper at [`TRUNK_ORACLE_DEFAULT`] (without
/// CARGO_TARGET_DIR, which would put it elsewhere).
const TRUNK_ORACLE_BUILD: &str = concat!("cd ", env!("CARGO_MANIFEST_DIR"), "/trunk-oracle && env -u CARGO_TARGET_DIR cargo build --release");

/// The helper this run uses, checked at startup ([`trunk_oracle_version`]);
/// `None` with `--no-trunk-oracle`.
static TRUNK_ORACLE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

fn trunk_oracle_path() -> PathBuf {
    std::env::var_os("TURSO_DIFF_TRUNK_ORACLE").map_or_else(|| PathBuf::from(TRUNK_ORACLE_DEFAULT), PathBuf::from)
}

fn unhex(h: &str) -> Vec<u8> {
    (0..h.len() / 2).filter_map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()).collect()
}

/// The helper's output on the database file `db` and the script `sql`
/// (files `trunk-N.*` in `dir`, removed after), within 30 s.
fn run_trunk_oracle(helper: &Path, db: &[u8], script: &str, dir: &Path) -> Result<String, String> {
    let n = SERIAL.fetch_add(1, Ordering::Relaxed);
    let (dbp, sqlp) = (dir.join(format!("trunk-{n}.db")), dir.join(format!("trunk-{n}.sql")));
    let go = || -> Result<String, String> {
        fs::write(&dbp, db).map_err(|e| format!("writing {}: {e}", dbp.display()))?;
        fs::write(&sqlp, script).map_err(|e| format!("writing {}: {e}", sqlp.display()))?;
        // The helper interrupts each statement after SQLite's budget; this
        // bounds the whole run too (a hang here was reported as a worker hang).
        let mut child = std::process::Command::new(helper)
            .arg(&dbp)
            .arg(&sqlp)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("running {}: {e}", helper.display()))?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() < Duration::from_secs(30) => thread::sleep(Duration::from_millis(20)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("timed out after 30 s".into());
                },
                Err(e) => return Err(format!("waiting: {e}")),
            }
        }
        let out = child.wait_with_output().map_err(|e| format!("reading its output: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let why = err.lines().find(|l| l.contains("panicked")).unwrap_or(err.trim());
            return Err(if why.is_empty() { format!("exited with {}", out.status) } else { format!("exited with {}: {why}", out.status) });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let r = go();
    let _ = fs::remove_file(&dbp);
    let _ = fs::remove_file(&sqlp);
    r
}

/// The helper at `helper` runs and is SQLite trunk: its `sqlite_version()`
/// and `sqlite_source_id()`; an error saying what is wrong otherwise.
fn trunk_oracle_version(helper: &Path, dir: &Path) -> Result<String, String> {
    if !helper.exists() {
        return Err(format!("the trunk-oracle helper {} does not exist", helper.display()));
    }
    let out = run_trunk_oracle(helper, &[], "SELECT sqlite_version(), sqlite_source_id();\n", dir).map_err(|e| format!("the trunk-oracle helper {} failed: {e}", helper.display()))?;
    let mut lines = out.lines();
    let row = match (lines.next(), lines.next()) {
        (Some("OK"), Some(row)) => row,
        _ => return Err(format!("the trunk-oracle helper {} gave unexpected output: {:?}", helper.display(), out.chars().take(200).collect::<String>())),
    };
    let cells: Vec<String> = row.split('\t').map(|c| String::from_utf8_lossy(&unhex(c.strip_prefix("T:").unwrap_or(""))).into_owned()).collect();
    let version = cells.first().cloned().unwrap_or_default();
    if version.is_empty() || version == rusqlite::version() {
        return Err(format!("the trunk-oracle helper {} reports SQLite {version:?}, not a trunk newer than {}", helper.display(), rusqlite::version()));
    }
    Ok(format!("SQLite {version} ({})", cells.get(1).map(|s| s.chars().take(30).collect::<String>()).unwrap_or_default()))
}

/// SQLite trunk's result for the finding's query, after the case's
/// statements and the queries before it; an error when the helper fails.
fn trunk_result(helper: &Path, input: &Input, k: usize, dir: &Path) -> Result<Result<Rows, String>, String> {
    let case = prepared(&input.0).map_err(|e| format!("preparing the case: {e}"))?;
    let text = run_trunk_oracle(helper, &case.file.0, &format!("{}{}", script(&case), queries_sql(&input.1, Some(k))), dir)?;
    let mut lines = text.lines();
    let head = lines.next().unwrap_or("");
    if let Some(e) = head.strip_prefix("E:") {
        return Ok(Err(e.to_string()));
    }
    if head != "OK" {
        return Err(format!("unexpected output {:?}", head.chars().take(80).collect::<String>()));
    }
    let rows = lines
        .map(|l| {
            l.split('\t')
                .filter(|c| !c.is_empty())
                .map(|c| match c.split_once(':') {
                    Some(("I", v)) => Cell::Int(v.parse().unwrap_or(0)),
                    Some(("R", v)) => Cell::Real(f64::from_bits(u64::from_str_radix(v, 16).unwrap_or(0))),
                    Some(("T", v)) => Cell::Text(String::from_utf8_lossy(&unhex(v)).into_owned()),
                    Some(("B", v)) => Cell::Blob(unhex(v)),
                    Some(("J", v)) => Cell::Json(String::from_utf8_lossy(&unhex(v)).into_owned()),
                    _ => Cell::Null,
                })
                .collect()
        })
        .collect();
    Ok(Ok(rows))
}

/// What SQLite trunk says about a finding ([`trunk_check`]).
enum Trunk {
    /// No query, or the oracle is off (`--no-trunk-oracle`).
    NotChecked,
    /// Trunk gives what Turso gave: a SQLite 3.50.4 behavior that newer
    /// SQLite changed to Turso's, not a Turso bug (an error in both counts
    /// as agreeing).
    Agrees,
    Differs,
    /// The helper failed: the finding is kept, and counted.
    Failed(String),
}

fn trunk_check(input: &Input, f: &Finding, dir: &Path) -> Trunk {
    let (Some(k), Some(turso), Some(Some(helper))) = (f.query, f.turso.clone(), TRUNK_ORACLE.get()) else { return Trunk::NotChecked };
    let trunk = match trunk_result(helper, input, k, dir) {
        Ok(t) => t,
        Err(e) => return Trunk::Failed(e),
    };
    let agrees = match (turso, trunk) {
        (Err(_), Err(_)) => true,
        (Ok(a), Ok(b)) => {
            let (a, b) = if f.ordered { (a, b) } else { (sorted(&a), sorted(&b)) };
            a == b || json_digits(&a) == json_digits(&b)
        },
        _ => false,
    };
    if agrees {
        Trunk::Agrees
    } else {
        Trunk::Differs
    }
}

/// Rows with every number written in a text cell rounded to 15 significant
/// digits: SQLite 3.50.4 writes JSON reals with 15, Turso with 17 (#8568).
fn json_digits(rows: &Rows) -> Rows {
    let round = |t: &str| -> String {
        let mut out = String::new();
        let b: Vec<char> = t.chars().collect();
        let mut i = 0;
        while i < b.len() {
            let start = i;
            if b[i] == '-' || b[i].is_ascii_digit() {
                let mut j = i + usize::from(b[i] == '-');
                let mut real = false;
                while j < b.len() && (b[j].is_ascii_digit() || b[j] == '.' || b[j] == 'e' || b[j] == 'E' || ((b[j] == '+' || b[j] == '-') && j > 0 && (b[j - 1] == 'e' || b[j - 1] == 'E'))) {
                    real |= b[j] == '.' || b[j] == 'e' || b[j] == 'E';
                    j += 1;
                }
                let tok: String = b[start..j].iter().collect();
                match (real, tok.parse::<f64>()) {
                    (true, Ok(x)) => out.push_str(&format!("{x:.14e}")),
                    _ => out.push_str(&tok),
                }
                i = j.max(start + 1);
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        out
    };
    rows.iter().map(|r| r.iter().map(|c| match c {
        Cell::Text(t) => Cell::Text(round(t)),
        Cell::Json(t) => Cell::Json(round(t)),
        c => c.clone(),
    }).collect()).collect()
}

/// The features of a query that tell differing rows apart: the families of
/// functions it calls and the constructs it uses. Appended to the
/// "rows-differ" signatures, so that one cause's examples don't take every
/// slot of the signature.
fn features(sql: &str) -> String {
    let l = sql.to_lowercase();
    let mut f: Vec<&str> = Vec::new();
    let has = |p: &str| l.contains(p);
    if has("json") || has("->") {
        f.push("json");
    }
    if ["date(", "time(", "datetime(", "julianday(", "unixepoch(", "strftime(", "timediff("].iter().any(|p| has(p)) {
        f.push("date");
    }
    if has(" over (") || has(" over(") {
        f.push("window");
    }
    for (p, n) in [("right join", "right-join"), ("full join", "full-join"), ("left join", "left-join")] {
        if has(p) {
            f.push(n);
        }
    }
    if has("with recursive") {
        f.push("recursive-cte");
    } else if l.trim_start().starts_with("with ") {
        f.push("cte");
    }
    for (p, n) in [(" union ", "compound"), (" intersect ", "compound"), (" except ", "compound")] {
        if has(p) && !f.contains(&n) {
            f.push(n);
        }
    }
    if has("group by") {
        f.push("group-by");
    }
    if has("distinct") {
        f.push("distinct");
    }
    if has("(select") {
        f.push("subquery");
    }
    if ["sum(", "total(", "avg("].iter().any(|p| has(p)) {
        f.push("sum");
    }
    if has("cast(") {
        f.push("cast");
    }
    if f.is_empty() {
        f.push("plain");
    }
    format!("[{}]", f.join(","))
}

/// A string literal in `sql` that is a date modifier with text after its
/// unit (`'+1 day '`, `'+1 day x'`): Turso #9837.
fn trailing_date_modifier(sql: &str) -> bool {
    let l = sql.to_lowercase();
    let units = ["days", "day", "hours", "hour", "minutes", "minute", "seconds", "second", "months", "month", "years", "year"];
    l.split('\'').skip(1).step_by(2).any(|lit| {
        let t = lit.trim_start();
        (t.starts_with('+') || t.starts_with('-') || t.starts_with(|c: char| c.is_ascii_digit()))
            && units.iter().any(|u| lit.find(u).is_some_and(|i| i + u.len() < lit.len() && !lit[i + u.len()..].starts_with('s')))
    })
}

/// `-0.0` standing alone (not the start of `-0.01`): Turso #9840.
fn negative_zero(s: &str) -> bool {
    s.match_indices("-0.0").any(|(i, m)| !s[i + m.len()..].starts_with(|c: char| c.is_ascii_digit() || c == 'e' || c == 'E'))
}

/// `rootpage` (a schema table's column) as the literal 0.
fn without_rootpage(sql: &str) -> String {
    let l = sql.to_lowercase();
    let mut out = String::new();
    let mut last = 0;
    for (i, m) in l.match_indices("rootpage") {
        let word = |c: char| c.is_alphanumeric() || c == '_';
        if !l[..i].ends_with(word) && !l[i + m.len()..].starts_with(word) {
            out.push_str(&sql[last..i]);
            out.push('0');
            last = i + m.len();
        }
    }
    out.push_str(&sql[last..]);
    out
}

/// A row value on the left of `[NOT] IN (SELECT ...)`: turso #9087.
fn row_value_in_subquery(l: &str) -> bool {
    l.match_indices(" in (select").chain(l.match_indices(" in(select")).any(|(i, _)| {
        let before = l[..i].trim_end().trim_end_matches(" not");
        if !before.ends_with(')') {
            return false;
        }
        // The `(` matching that `)`, and a comma at the top level between.
        let (mut depth, mut comma) = (0, false);
        for c in before.chars().rev() {
            match c {
                ')' => depth += 1,
                '(' if depth == 1 => return comma,
                '(' => depth -= 1,
                ',' if depth == 1 => comma = true,
                _ => {},
            }
        }
        false
    })
}

/// A simple CASE (`CASE x WHEN ...`), whose comparisons Turso makes without
/// SQLite's affinity rules (turso #8225).
fn simple_case(l: &str) -> bool {
    l.match_indices("case ").any(|(i, m)| !l[i + m.len()..].trim_start().starts_with("when"))
}

/// Filed issues fixed on the Turso commit under test (checked by rerunning
/// each repro): a finding a rule would attribute to one of these is a
/// regression or another bug, so it is reported, not labelled known.
const FIXED: &[&str] = &["9839", "9884", "9887", "9889", "9921", "9924", "9925", "9926", "9927", "9932", "9934", "9935"];

/// Rule labels for causes not yet filed upstream: the local draft's
/// directory under `case-studies/turso/bugs/` (or `bugs/candidates/`).
const DRAFT_JSON_BLOB: &str = "draft-json-blob-arg-jsonb-detection";
const DRAFT_DEFERRED_SEEK: &str = "draft-multi-index-or-stale-deferred-seek";
const CANDIDATE_JSONB_UTF8: &str = "candidate-jsonb-text-non-utf8-rejected";

/// How a rule's label is shown: `turso #N` for a filed issue, the draft's
/// name otherwise.
fn label(issue: &str) -> String {
    if issue.chars().all(|c| c.is_ascii_digit()) {
        format!("turso #{issue}")
    } else {
        issue.to_string()
    }
}

/// The notes [`jsonb_notes`] writes, which rules read.
const NOTE_JSONB_NOT_UTF8: &str = "note: a JSONB blob (json_valid(X, 8) = 1) holds a text element that is not UTF-8 (candidate jsonb-text-non-utf8-rejected shape)\n";
const NOTE_JSONB_WHITESPACE: &str = "note: a JSONB blob (json_valid(X, 8) = 1) starts with a whitespace byte 09/0A/0D/20 (draft json-blob-arg-jsonb-detection shape)\n";

/// Whether a JSONB value holds a TEXT, TEXTJ, TEXT5 or TEXTRAW element
/// whose payload is not UTF-8 (SQLite's strict check works on bytes, so
/// `json_valid(X, 8)` accepts it; Turso's other JSON functions do not).
fn jsonb_text_not_utf8(b: &[u8]) -> bool {
    let mut i = 0;
    while i < b.len() {
        let (ty, code) = (b[i] & 0x0f, b[i] >> 4);
        let (len, head) = match code {
            0..=11 => (code as usize, 1),
            12..=15 => {
                let n = 1usize << (code - 12);
                if i + 1 + n > b.len() {
                    return false;
                }
                (b[i + 1..i + 1 + n].iter().fold(0usize, |a, &x| a.wrapping_shl(8) | x as usize), 1 + n)
            },
            _ => unreachable!(),
        };
        let start = i + head;
        let Some(end) = start.checked_add(len).filter(|&e| e <= b.len()) else { return false };
        match ty {
            // An array or object: its elements follow, as its payload.
            11 | 12 => i = start,
            7..=10 => {
                if std::str::from_utf8(&b[start..end]).is_err() {
                    return true;
                }
                i = end;
            },
            _ => i = end,
        }
    }
    false
}

/// Notes on the BLOB values in the database's tables that SQLite takes for
/// JSONB and Turso reads differently (see [`NOTE_JSONB_NOT_UTF8`] and
/// [`NOTE_JSONB_WHITESPACE`]): for rules on queries failing with a JSON
/// parse error.
fn jsonb_notes(db: &Connection) -> String {
    let (mut utf8, mut space) = (false, false);
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND upper(sql) NOT LIKE 'CREATE VIRTUAL%'")
        .and_then(|mut st| st.query_map([], |r| r.get::<_, String>(0)).map(|rows| rows.filter_map(Result::ok).collect()))
        .unwrap_or_default();
    let q = |n: &str| format!("\"{}\"", n.replace('"', "\"\""));
    for t in tables {
        let cols: Vec<String> = db
            .prepare("SELECT name FROM pragma_table_info(?1)")
            .and_then(|mut st| st.query_map([&t], |r| r.get::<_, String>(0)).map(|rows| rows.filter_map(Result::ok).collect()))
            .unwrap_or_default();
        for c in cols {
            let sql = format!("SELECT {c} FROM {t} WHERE typeof({c}) = 'blob' AND json_valid({c}, 8) LIMIT 1000", c = q(&c), t = q(&t));
            let Ok(mut st) = db.prepare(&sql) else { continue };
            let Ok(mut rows) = st.query([]) else { continue };
            while let Ok(Some(r)) = rows.next() {
                if let Ok(ValueRef::Blob(b)) = r.get_ref(0) {
                    space |= matches!(b.first(), Some(9 | 10 | 13 | 32));
                    utf8 |= jsonb_text_not_utf8(b);
                }
            }
        }
    }
    format!("{}{}", if utf8 { NOTE_JSONB_NOT_UTF8 } else { "" }, if space { NOTE_JSONB_WHITESPACE } else { "" })
}

/// `name` as written in a statement (quoted, bracketed, schema-qualified),
/// as a bare lowercase name.
fn bare_name(tok: &str) -> String {
    let t = tok.trim_end_matches([';', ',']);
    let t = t.rsplit('.').next().unwrap_or(t);
    t.trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']' || c == '\'').to_lowercase()
}

/// Every single statement of `stmts` (a list entry can hold several),
/// lowercased.
fn single_statements(stmts: &[String]) -> Vec<String> {
    stmts.iter().flat_map(|s| split_statements(s)).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
}

/// `ALTER TABLE name rest`: the bare name and the rest's first word.
fn alter_target(s: &str) -> Option<(String, String)> {
    let w: Vec<&str> = s.split_whitespace().collect();
    (w.len() >= 4 && w[0].eq_ignore_ascii_case("alter") && w[1].eq_ignore_ascii_case("table")).then(|| (bare_name(w[2]), w[3].to_lowercase()))
}

/// The tables a column was dropped from (`ALTER TABLE t DROP [COLUMN] c`).
fn dropped_column_tables(stmts: &[String]) -> Vec<String> {
    single_statements(stmts)
        .iter()
        .filter(|s| !s.contains(" drop constraint "))
        .filter_map(|s| alter_target(s).filter(|(_, verb)| verb == "drop").map(|(t, _)| t))
        .collect()
}

/// A column default of the minimum integer: Turso reads it back as a REAL
/// (turso #10391). RENAME TABLE / COLUMN move the column, so the whole
/// statement list counts.
fn min_int_default(stmts: &[String]) -> bool {
    stmts.iter().any(|s| {
        let c: String = s.to_lowercase().chars().filter(|c| !c.is_whitespace() && *c != '(').collect();
        c.contains("default-9223372036854775808")
    })
}

/// A string literal holding a float with a three-digit exponent
/// (`'-8.5e260'`), compared with a REAL column: Turso's text-to-REAL
/// conversion gives another double there (turso #9849).
fn big_exponent_text(l: &str) -> bool {
    l.split('\'').skip(1).step_by(2).any(|lit| {
        let t = lit.trim();
        t.parse::<f64>().is_ok() && t.split_once('e').is_some_and(|(_, e)| e.trim_start_matches(['+', '-']).len() >= 3)
    })
}

/// The byte offset where a statement's WITH clause ends (0 without one),
/// and the names of its CTEs.
fn with_clause(l: &str) -> (usize, Vec<String>) {
    let Some(mut rest) = l.trim_start().strip_prefix("with ") else { return (0, Vec::new()) };
    rest = rest.trim_start();
    if let Some(r) = rest.strip_prefix("recursive ") {
        rest = r;
    }
    let mut names = Vec::new();
    loop {
        rest = rest.trim_start();
        let name_end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '"')).unwrap_or(rest.len());
        names.push(rest[..name_end].trim_matches('"').to_string());
        // The body: the first top-level `(` after AS, to its match.
        let Some(as_at) = rest.find(" as ") else { break };
        let Some(open) = rest[as_at..].find('(').map(|k| as_at + k) else { break };
        let mut depth = 0;
        let mut close = None;
        for (k, c) in rest[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + k);
                        break;
                    }
                },
                _ => {},
            }
        }
        let Some(close) = close else { break };
        rest = &rest[close + 1..];
        match rest.trim_start().strip_prefix(',') {
            Some(r) => rest = r,
            None => break,
        }
    }
    (l.len() - rest.len(), names)
}

/// The spans of the left operands of `IN (` in `l`: the parenthesized
/// expression right before each ` in (` / ` not in (`.
fn in_left_operands(l: &str) -> Vec<(usize, usize)> {
    let b = l.as_bytes();
    let mut out = Vec::new();
    for (i, _) in l.match_indices(" in (") {
        let before = l[..i].trim_end();
        let before = before.strip_suffix(" not").unwrap_or(before).trim_end();
        if !before.ends_with(')') {
            continue;
        }
        let end = before.len();
        let mut depth = 0;
        for k in (0..end).rev() {
            match b[k] {
                b')' => depth += 1,
                b'(' => {
                    depth -= 1;
                    if depth == 0 {
                        out.push((k, end));
                        break;
                    }
                },
                _ => {},
            }
        }
    }
    out
}

/// The CTE `name` is read only inside the left operand of an `IN (...)`
/// (turso #8655): every use after the WITH clause lies in one.
fn cte_only_in_in_left(l: &str, name: &str) -> bool {
    let (body_at, _) = with_clause(l);
    let spans = in_left_operands(l);
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let uses: Vec<usize> = l
        .match_indices(name)
        .map(|(i, _)| i)
        .filter(|&i| i >= body_at && !l[..i].ends_with(word) && !l[i + name.len()..].starts_with(word))
        .collect();
    !uses.is_empty() && uses.iter().all(|&i| spans.iter().any(|&(a, b)| a < i && i < b))
}

/// How many `(SELECT` subqueries enclose byte `pos` of `l`.
fn select_depth(l: &str, pos: usize) -> usize {
    let mut stack: Vec<bool> = Vec::new();
    for (k, c) in l[..pos].char_indices() {
        match c {
            '(' => stack.push(l[k + 1..].trim_start().starts_with("select")),
            ')' => {
                stack.pop();
            },
            _ => {},
        }
    }
    stack.iter().filter(|s| **s).count()
}

/// A `json_valid(X, F)` with F in 8..=15: a JSONB guard (the generator's
/// `CASE WHEN json_valid(c, 10) THEN c END`).
fn jsonb_guard(l: &str) -> bool {
    l.match_indices("json_valid(").any(|(i, m)| {
        let rest = &l[i + m.len()..];
        let mut depth = 1;
        let mut last_comma = None;
        for (k, c) in rest.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return last_comma.is_some_and(|at: usize| rest[at + 1..k].trim().parse::<u32>().is_ok_and(|f| (8..=15).contains(&f)));
                    }
                },
                ',' if depth == 1 => last_comma = Some(k),
                _ => {},
            }
        }
        false
    })
}

/// A scalar subquery over `json_tree(..)` / `json_each(..)` of an outer
/// table's column (`a0.c4`): Turso runs it in that table's loop.
fn correlated_json_source(l: &str) -> bool {
    ["(select count(distinct value) from json_tree(", "(select count(*) from json_tree(", "(select count(distinct value) from json_each(", "(select count(*) from json_each("].iter().any(|p| {
        l.match_indices(p).any(|(i, m)| {
            let arg: String = l[i + m.len()..].chars().take(80).collect();
            let b = arg.as_bytes();
            (0..b.len().saturating_sub(2)).any(|k| b[k] == b'a' && b[k + 1].is_ascii_digit() && (k == 0 || !b[k - 1].is_ascii_alphanumeric()) && arg[k + 1..].trim_start_matches(|c: char| c.is_ascii_digit()).starts_with('.'))
        })
    })
}

/// What SQLite gave, from a finding's detail (`SQLite: ...`).
fn sqlite_part(detail: &str) -> &str {
    let head = detail.split("\nTurso:").next().unwrap_or("");
    head.split("\nSQLite: ").nth(1).unwrap_or("")
}

/// SQLite returned no rows, or one row of a NULL or 0 aggregate: a query
/// whose failing expression SQLite never evaluates.
fn sqlite_empty(detail: &str) -> bool {
    let s = sqlite_part(detail).trim_end();
    s.starts_with("0 rows") || s == "1 rows\n  NULL" || s == "1 rows\n  0"
}

fn known(f: &Finding, sql: &str, stmts: &[String]) -> Option<&'static str> {
    known_rules(f, sql, stmts).into_iter().find(|i| !FIXED.contains(i))
}

/// The filed Turso issue (or local draft, [`label`]) a finding matches, by
/// its signature, its query (or statement), and the input's statements
/// (`stmts`; for a statement error, those before it count): matched
/// findings keep one example each under `findings/known/`, unshrunk. The
/// rules are deliberately narrow; an example per issue still shows what
/// they catch. [`known`] takes the first hit, so the rules keyed on a
/// signature come before those keyed on words in the query.
fn known_rules(f: &Finding, sql: &str, stmts: &[String]) -> Vec<&'static str> {
    let mut hits: Vec<&'static str> = Vec::new();
    let sig = f.signature.as_str();
    // A statement error's text is "statement K: <sql>;\nSQLite: ...": the
    // statement, which can span lines (a newline in a string literal).
    let l = match sig.starts_with("statement-error") {
        true => sql.split("\nSQLite:").next().unwrap_or("").split_once(": ").map_or(sql, |(_, s)| s).to_lowercase(),
        false => sql.to_lowercase(),
    };
    let any = |ps: &[&str]| ps.iter().any(|p| l.contains(p));
    let joins = any(&["right join", "full join", "right outer join", "full outer join"]);
    let rows = sig.starts_with("rows-differ");
    let query = sig.starts_with("query-error") || sig.starts_with("query-accepted");
    if sig.starts_with("transaction-state") {
        hits.push("6864");
    }
    if sig.starts_with("panic") {
        if sig.contains("index out of bounds") && l.contains("->") {
            hits.push("9836");
        }
        if sig.contains("duplicate cursor key") && (joins || any(&["json_each", "json_tree", "generate_series"])) {
            hits.push("9847");
        }
        if sig.contains("cursor id N is None") && joins {
            hits.push("9850");
        }
        if sig.contains("cursor id N is None") && (l.contains("materialized") || l.trim_start().starts_with("with")) {
            hits.push("9882");
        }
        if sig.contains("WITHOUT ROWID table has no secondary index") {
            hits.push("9852");
        }
        if sig.contains("non-REPLACE constraint after REPLACE constraint") || sig.contains("constraints must have all REPLACE entries at the end") {
            hits.push("8718");
        }
        if sig.contains("GroupConcat accumulator is Text") {
            hits.push("10352");
        }
        if any(&[" over ", " over("]) && (sig.contains("must be a buffer") || sig.contains("first_value") || sig.contains("lag/lead") || sig.contains("nth_value arg") || sig.contains("must be a subquery column ref")) {
            hits.push("8336");
        }
        return hits;
    }
    if sig.starts_with("statement-error") {
        // The statements before the failing one ("statement K: ...").
        let k = sql.strip_prefix("statement ").and_then(|s| s.split(':').next()).and_then(|k| k.parse::<usize>().ok()).unwrap_or(stmts.len());
        let before = &stmts[..k.min(stmts.len())];
        let ls = l.trim_start().strip_prefix("explain ").unwrap_or(l.trim_start());
        if sig.contains("VACUUM INTO path cannot be empty") || sig.contains("output file already exists: :memory:") {
            hits.push("10341");
        }
        // A double-quoted string in an index expression or partial-index WHERE.
        if (sig.contains("invalid expression in CREATE INDEX") || sig.contains("cannot use aggregate, window functions or reference other tables in WH")) && l.contains("create") && l.contains("index") && l.contains('"') {
            hits.push("10337");
        }
        if sig.contains("no such table or index") && l.contains("analyze") {
            hits.push("10340");
        }
        if sig.contains("syntax error") && l.trim_start().starts_with("alter table") && f.detail.contains("names a WITHOUT ROWID table") {
            hits.push("9572");
        }
        // An ALTER of a STRICT WITHOUT ROWID table writes "WITHOUT ROWID
        // STRICT" (no comma); the next ALTER, of any table, rereads it.
        if f.exact.contains("near \"STRICT\": syntax error") && l.trim_start().starts_with("alter table") {
            hits.push("9572");
        }
        // DROP COLUMN of a column with a column-level REFERENCES: SQLite drops
        // the constraint with it.
        if sig.contains("after drop column: unknown column") && (sig.contains("in foreign key defini") || f.exact.contains("in foreign key definition")) {
            hits.push("9077");
        }
        // DROP COLUMN on a WITHOUT ROWID table leaves the column's values in
        // each record: a column added back reads them (NULL fails NOT NULL),
        // and so does `INSERT .. SELECT * FROM` it.
        if sig.contains("NOT NULL constraint failed") && f.detail.contains("names a WITHOUT ROWID table") {
            let dropped = dropped_column_tables(before);
            let added_back = alter_target(ls).is_some_and(|(t, verb)| verb == "add" && dropped.contains(&t));
            let read_back = split_statements(&l).iter().any(|s| {
                let s = s.trim_start().strip_prefix("explain ").unwrap_or(s.trim_start());
                (s.starts_with("insert") || s.starts_with("replace")) && s.split_once(" select ").is_some_and(|(_, sel)| dropped.iter().any(|t| mentions(sel, t)))
            });
            if added_back || read_back {
                hits.push("9071");
            }
        }
        // An INSERT that a DELETE/UPDATE over an OR of an index branch and a
        // rowid branch should have made room for (draft, not filed).
        if sig.contains("UNIQUE constraint failed") && !f.detail.contains("WITHOUT ROWID") {
            let target = ls.split_whitespace().skip_while(|w| *w != "into").nth(1).map(bare_name);
            if let Some(t) = target.filter(|_| ls.starts_with("insert") || ls.starts_with("replace")) {
                let deleted = single_statements(before).iter().any(|s| {
                    let s = s.strip_prefix("explain ").unwrap_or(s);
                    let w: Vec<&str> = s.split_whitespace().collect();
                    let table = match w.as_slice() {
                        ["delete", "from", n, ..] => Some(bare_name(n)),
                        ["update", n, ..] => Some(bare_name(n)),
                        _ => None,
                    };
                    table.as_deref() == Some(t.as_str()) && s.split_once(" where ").is_some_and(|(_, w)| w.contains(" or "))
                });
                if deleted {
                    hits.push(DRAFT_DEFERRED_SEEK);
                }
            }
        }
        if sig.contains("UNIQUE constraint failed") && f.detail.contains("WITHOUT ROWID") {
            hits.push("6749");
        }
        if sig.contains("cannot store") && l.contains("or ignore") {
            hits.push("10392");
        }
        if sig.contains("cannot store REAL value in INT") && min_int_default(before) {
            hits.push("10391");
        }
        if (sig.contains("no such table") || sig.contains("no such column")) && l.contains("on conflict") {
            hits.push("10393");
        }
        if sig.contains("cannot drop column") && sig.contains("referenced in VIEW") {
            hits.push("4009");
        }
        if sig.contains("after rename column") || (sig.contains("error in trigger") && any(&["rename", "drop column"])) {
            hits.push("7644");
        }
        return hits;
    }
    // Rules keyed on a query's error message, before any keyed on words in
    // the query: [`known`] takes the first match.
    if query {
        if any(&["sqlite_master.", "sqlite_schema.", "sqlite_temp_master.", "sqlite_temp_schema."]) && sig.contains("no such table: sqlite_") {
            hits.push("10353");
        }
        if sig.contains("root path should be text") {
            hits.push("9838");
        }
        if sig.contains("second argument to likelihood()") {
            hits.push("10338");
        }
        if sig.contains("insufficient registers allocated for expression vector") && l.contains("between") {
            hits.push("9051");
        }
        if sig.contains("Too many arguments for pragma_quick_check") || sig.contains("Too many arguments for pragma_integrity_check") {
            hits.push("10339");
        }
        // A CTE read only in the left operand of IN (turso #8655), else one
        // read in a subquery of a JOIN's ON clause (turso #9073).
        if sig.contains("no such table: xN") {
            let name = f.exact.rsplit("no such table: ").next().unwrap_or("").trim().to_lowercase();
            if !name.is_empty() && cte_only_in_in_left(&l, &name) {
                hits.push("8655");
            }
            if l.contains(" on ") {
                hits.push("9073");
            }
        }
        // A BLOB SQLite's json_valid(X, 8) accepts, holding a text element
        // that is not UTF-8: Turso's JSON functions reject it (candidate).
        if sig.contains("Parse error: malformed JSON") && f.detail.contains(NOTE_JSONB_NOT_UTF8.trim_end()) && jsonb_guard(&l) {
            hits.push(CANDIDATE_JSONB_UTF8);
        }
        // A JSONB BLOB starting with 09/0A/0D/20 (X'0A', an empty TEXTRAW):
        // Turso skips the byte as whitespace (draft).
        if sig.contains("Unexpected end of input") && f.detail.contains(NOTE_JSONB_WHITESPACE.trim_end()) {
            hits.push(DRAFT_JSON_BLOB);
        }
        // An expression SQLite never evaluates (no rows reach it), which
        // Turso evaluates once up front: a scalar subquery, a literal-only
        // expression, or a correlated subquery run in the loop of its outer
        // table, before an empty inner table's loop (turso #9851).
        let evaluated = sig.contains("integer overflow") || sig.contains("malformed JSON") || sig.contains("bad JSON path");
        let literal = if sig.contains("integer overflow") {
            l.contains("(select") || l.contains("abs(-9223372036854775808)")
        } else {
            // A JSON literal: the generator's guarded BLOB literal
            // (`CASE WHEN json_valid(x'..', 8) THEN x'..' END`) or a text
            // literal right before `->`/`->>`.
            any(&["json_valid(x'", "}' ->", "]' ->", "}'->", "]'->"])
        };
        if sig.starts_with("query-error") && evaluated && sqlite_empty(&f.detail) && literal {
            hits.push("9851");
        }
        if sig.starts_with("query-error") && (sig.contains("malformed JSON") || sig.contains("bad JSON path")) && l.contains(" join ") && correlated_json_source(&l) {
            hits.push("9851");
        }
        // An aggregate of outer columns two or more subqueries down: SQLite
        // says misuse, Turso aggregates it (turso #9519).
        if sig.starts_with("query-accepted: misuse of aggregate") {
            let func = f.exact.split("misuse of aggregate: ").nth(1).and_then(|s| s.split('(').next()).unwrap_or("").trim().to_lowercase();
            if !func.is_empty() && l.match_indices(&format!("{func}(")).any(|(i, _)| select_depth(&l, i) >= 2) {
                hits.push("9519");
            }
        }
    }
    if rows {
        if min_int_default(stmts) && (f.detail.contains("-9.22337203685478e+18") || f.detail.contains("-9.223372036854776e18")) {
            hits.push("10391");
        }
        // A REAL column against text holding a float with a three-digit
        // exponent, or against a string_agg of the column (its text).
        if big_exponent_text(&l) || (sig.contains("row count") && any(&["= (select string_agg(", "= (select group_concat("])) {
            hits.push("9849");
        }
    }
    // Rules keyed on words in the query: for differing rows (an error has
    // its own rules above).
    if rows {
        if any(&["sqlite_master.", "sqlite_schema.", "sqlite_temp_master.", "sqlite_temp_schema."]) && sig.contains("no such table: sqlite_") {
            hits.push("10353");
        }
        if any(&["pragma_quick_check", "pragma_integrity_check"]) {
            hits.push("10339");
        }
        let turso_out = f.detail.split("\nTurso:").nth(1).unwrap_or("");
        // An expression index's column: cid -2 and no name in SQLite.
        if any(&["pragma_index_info", "pragma_index_xinfo"]) && turso_out.contains("(") {
            hits.push("5769");
        }
        if l.contains("pragma_index_xinfo") {
            hits.push("10354");
        }
        // SQLite's ANALYZE creates sqlite_stat1 (and sqlite_stat4) even with
        // nothing to analyze.
        if l.contains("sqlite_schema") || l.contains("sqlite_master") || l.contains("pragma_table_list") {
            let sqlite_out = f.detail.split("\nTurso:").next().unwrap_or("");
            if sqlite_out.contains("sqlite_stat1") && !turso_out.contains("sqlite_stat1") {
                hits.push("10355");
            }
        }
        if l.contains("replace(") && l.contains(", null)") {
            hits.push("10426");
        }
        if l.contains("soundex(") {
            hits.push("10356");
        }
        if f.detail.contains("#10355 shape") && any(&["sqlite_schema", "sqlite_master", "pragma_table_list"]) {
            hits.push("10355");
        }
        if f.detail.contains("#9071 shape") && f.detail.contains("reads a WITHOUT ROWID table") {
            hits.push("9071");
        }
        if f.detail.contains("#10397 shape") && any(&["pragma_table_list", "pragma_table_info", "pragma_table_xinfo"]) {
            hits.push("10397");
        }
        if f.detail.contains("#5762 shape") && any(&["pragma_table_info", "pragma_table_xinfo"]) {
            hits.push("5762");
        }
        if row_value_in_subquery(&l) {
            hits.push("9087");
        }
    }
    if rows || query {
        if sig.contains("ORDER BY term does not match any column") && any(&["isnull", "notnull", "is null", "not null"]) {
            hits.push("10395");
        }
        if sig.contains("no such column: main.") || sig.contains("no such column: temp.") {
            hits.push("10396");
        }
        if sig.contains("subquery not found") {
            hits.push("10389");
        }
    }
    if rows {
        if l.contains("concat_ws") {
            hits.push("10394");
        }
        if any(&["sqlite_schema", "sqlite_master"]) && f.detail.contains("CREATE ") {
            hits.push("10398");
        }
        if any(&["pragma_index_list", "pragma_index_info", "pragma_index_xinfo"]) && f.detail.contains("reads a WITHOUT ROWID table") {
            hits.push("10399");
        }
    }
    if sig.contains("number too large to fit in target type") && any(&["json_each", "json_tree"]) {
        hits.push("9839");
    }
    if sig.contains("failed to determine json error position") {
        hits.push("9841");
    }
    if sig.contains("all before-loop subqueries should have already been emitted") {
        hits.push("9848");
    }
    if sig.contains("table not found") && l.contains(" over ") {
        hits.push("9883");
    }
    // A HAVING subquery reading a GROUP BY column (turso #8221), before the
    // broader GROUP BY + HAVING rule below.
    if rows && l.contains("group by") && l.split("having").nth(1).is_some_and(|h| h.contains("(select")) {
        hits.push("8221");
    }
    if ((sig.starts_with("query-error") && !sig.contains("Parse error")) || rows) && l.contains("group by") && l.contains("having") {
        hits.push(if sig.starts_with("query-error") { "9880" } else { "9887" });
    }
    if sig.starts_with("query-error: integer overflow") && l.contains("(select") {
        hits.push("9851");
    }
    if f.detail.contains("#6749 shape") && (sig.contains("UNIQUE constraint failed") || (rows && f.detail.contains("reads a WITHOUT ROWID table"))) {
        hits.push("6749");
    }
    if rows && l.contains(" join ") && f.detail.contains("reads a WITHOUT ROWID table") && (sig.contains("row count") || l.contains("count(")) {
        hits.push("9081");
    }
    if sig.starts_with("query-error: Parse error: no such column") && l.trim_start().starts_with("with") {
        hits.push("9842");
    }
    if sig.contains("UNIQUE constraint failed") && f.detail.to_lowercase().contains("without rowid") {
        hits.push("6749");
    }
    if rows || query {
        // A JSON parse error: the rules on non-JSON functions (dates,
        // strftime, timediff, group_concat) and on how JSON is printed don't
        // explain it, nor those on JSON functions the query doesn't call.
        let json_error = query && ["malformed JSON", "Unexpected end of input", "Unexpected character", "JSON cannot hold BLOB"].iter().any(|p| sig.contains(p));
        if trailing_date_modifier(sql) && !json_error {
            hits.push("9837");
        }
        if any(&["json_insert(", "jsonb_insert("]) && l.contains("'$'") {
            hits.push("9924");
        }
        if any(&["json_set((case when json_valid", "jsonb_set((case when json_valid", "json_insert((case when json_valid", "jsonb_insert((case when json_valid", "json_replace((case when json_valid", "jsonb_replace((case when json_valid"]) {
            hits.push("9844");
        }
        if any(&["'24:", "'+24:", "'-24:"]) && !json_error {
            hits.push("9921");
        }
        if simple_case(&l) && rows {
            hits.push("8225");
        }
        if l.contains("json_pretty") && rows {
            hits.push("9885");
        }
        if l.contains("timediff") && !json_error {
            hits.push("9886");
        }
        if l.contains("strftime") && !json_error {
            hits.push(if l.contains("%j") { "9888" } else { "9884" });
        }
        if negative_zero(&f.detail) && any(&["json", "->"]) && rows {
            hits.push("9840");
        }
        if any(&["json_patch", "jsonb_patch"]) {
            hits.push("9843");
        }
        if any(&["'$[#]'", "'$[#"]) && any(&["json_set", "jsonb_set", "json_insert", "jsonb_insert"]) {
            hits.push("8538");
        }
        let json_edit = any(&["json_set(", "jsonb_set(", "json_insert(", "jsonb_insert(", "json_replace(", "jsonb_replace(", "json_remove(", "jsonb_remove("]);
        if any(&["json_set(null", "jsonb_set(null", "json_insert(null", "jsonb_insert(null", "json_replace(null", "jsonb_replace(null", "json_remove(null", "jsonb_remove(null"]) || (any(&["', null", "$', null"]) && (rows || json_edit)) {
            hits.push("9844");
        }
        if json_edit && (sig.contains("JSON cannot hold BLOB") || f.detail.contains("NULL")) {
            hits.push("9844");
        }
        if l.contains("->") && (l.contains("'[") || l.contains("> '[")) {
            hits.push("9845");
        }
        if sig.contains("reals within 1e-12") {
            hits.push("9849");
        }
        if l.contains("json_error_position") {
            hits.push("9841");
        }
        if l.contains("quote(") && (f.detail.contains("Inf") || f.detail.contains("e+999")) {
            hits.push("8567");
        }
        if any(&["json_each", "json_tree"]) && l.trim_start().starts_with("with") && rows {
            hits.push("10342");
        }
        if sig.contains("malformed JSON") && (f.detail.contains("//") || l.contains("//")) {
            hits.push("10343");
        }
        // #10352 is the group_concat window panic; differing rows only.
        if l.contains("group_concat") && l.contains(" over ") && f.detail.contains("NULL") && rows {
            hits.push("10352");
        }
        let turso_out = f.detail.split("\nTurso:").nth(1).unwrap_or("");
        if l.contains("||") && (turso_out.contains("x'") || sig.contains("JSON cannot hold BLOB")) {
            hits.push("6089");
        }
    }
    hits
}

/// Run one input on both engines.
fn run_input(input: &Input, path: &Path, experimental: bool) -> Outcome {
    let (case, queries) = input;
    let mut out = Outcome::default();
    let case = &match prepared(case) {
        Ok(c) => c,
        Err(e) => {
            out.skips.push(format!("sqlite setup: {}", normalize(&e)));
            return out;
        },
    };
    // The generator's model of the statements, for which queries have a total
    // order, and against which the queries render ([`case_schema`]): of the
    // input's own case, since the generator's reader does not read most
    // prepared files back. Its write model panics on some statement lists
    // left after a failing statement was dropped (see the worker loop): skip
    // those.
    let Some(schema) = case_schema(&input.0) else {
        out.skips.push("generator model rejects the statements left after a drop".into());
        return out;
    };
    // Turso reads UTF-8 databases only (documented); skip the others before
    // running anything. The text encoding is the header's 4 bytes at 56.
    let bytes = &case.file.0;
    if bytes.len() >= 60 && bytes[56..60] != [0, 0, 0, 1] && bytes[56..60] != [0, 0, 0, 0] {
        out.skips.push("utf-16 database (turso reads UTF-8 only)".into());
        return out;
    }
    let sdb = match open_sqlite(case) {
        Ok(db) => db,
        Err(e) => {
            out.skips.push(format!("sqlite setup: {}", normalize(&e)));
            out.failed_statement = statement_index(&e);
            return out;
        },
    };
    let t = match open_turso(case, path, experimental) {
        Ok(t) => t,
        Err(e) => {
            if unsupported(&e) {
                out.skips.push(format!("turso unsupported (open): {}", normalize(&e)));
            } else {
                out.findings.push(Finding { signature: format!("open-error: {}", normalize(&e)), detail: format!("Turso failed to open the file: {e}"), exact: e.clone(), query: None, ..Finding::default() });
            }
            return out;
        },
    };
    let mut notes = affinity_order_note(&sdb);
    let wr_ever = without_rowid_ever(&sdb, case);
    for (k, s) in case.statements.iter().enumerate() {
        let sql = s.to_string();
        if let Err(e) = turso_exec(&t, &sql) {
            if unsupported(&e) {
                out.skips.push(format!("turso unsupported (statement): {}", normalize(&e)));
                out.failed_statement = Some(k);
            } else if documented_without_rowid(&e) && (writes_without_rowid(&wr_ever, &sql) || writes_without_rowid(&wr_ever, &e)) {
                out.skips.push(format!("turso documented (WITHOUT ROWID): {}", normalize(&e)));
                out.failed_statement = Some(k);
            } else {
                out.failed_statement = Some(k);
                let wr = if writes_without_rowid(&wr_ever, &sql) { "note: the statement names a WITHOUT ROWID table\n" } else { "" };
                out.findings.push(Finding {
                    signature: format!("statement-error: {}", normalize(&e)),
                    detail: format!("statement {k}: {sql};\nSQLite: ok\nTurso: error: {e}\n{notes}{wr}"),
                    exact: e.clone(),
                    query: None,
                    ..Finding::default()
                });
            }
            return out;
        }
    }
    if let Some(f) = transaction_state(&sdb, &t, "the statements", None) {
        out.findings.push(f);
        return out;
    }
    notes.push_str(&statement_notes(&sdb, &t, case, &wr_ever));
    let wr_tables = without_rowid_tables(&sdb);
    // [`jsonb_notes`], made once, for the first query Turso fails.
    let mut jsonb: Option<String> = None;
    for (qi, q) in queries.iter().enumerate() {
        let sql = q.sql();
        out.queries += 1;
        // Function sets differ by design (Turso's extensions; SQLite's
        // build options), and so do the rows describing them.
        // page_count and freelist_count depend on how each engine lays out
        // the file.
        if ["pragma_function_list", "pragma_page_count", "pragma_freelist_count"].iter().any(|p| sql.to_lowercase().contains(p)) {
            out.skips.push("query reads pragma_function_list, page_count or freelist_count".into());
            continue;
        }
        // rootpage depends on how each engine lays out the file: both read 0.
        let sql = without_rootpage(&sql);
        let s = sqlite_query(&sdb, &sql);
        // SQLite over its budget: don't run Turso on it either (a slow query
        // in both engines is not a Turso hang).
        if matches!(&s, Err(e) if e.contains("interrupted")) {
            out.skips.push("sqlite over its time budget (query not run on turso)".into());
            continue;
        }
        let r = turso_query(&t, &sql);
        let lsql = sql.to_lowercase();
        let note = format!(
            "{notes}{}",
            if wr_tables.iter().any(|w| lsql.contains(&w.to_lowercase())) { "note: the query reads a WITHOUT ROWID table\n" } else { "" }
        );
        if let Some(mut f) = transaction_state(&sdb, &t, &sql, Some(qi)) {
            f.detail = format!("{}\nSQLite: {}\nTurso: {}", f.detail, show(&s), show(&r));
            out.findings.push(f);
            return out;
        }
        let (s, r) = match (s, r) {
            (Err(_), Err(_)) => continue,
            (Ok(_), Err(e)) if e.contains("no such function: json_array_insert") || e.contains("no such function: jsonb_array_insert") => {
                out.skips.push("json_array_insert: in neither engine at these versions".into());
                continue;
            },
            // SQLITE_ENABLE_STAT4 (this build, not Turso's target) evaluates a
            // function used as an index range bound with the column's affinity.
            (Err(e), Ok(_)) if e.contains("labels must be TEXT") => {
                out.skips.push("sqlite STAT4 build quirk: json_object label affinity".into());
                continue;
            },
            (Ok(_), Err(e)) if unsupported(&e) => {
                out.skips.push(format!("turso unsupported (query): {}", normalize(&e)));
                continue;
            },
            // Newer than SQLite 3.50.4, which Turso already has.
            (Err(e), Ok(_)) if e.contains("no such function: json_array_insert") || e.contains("no such function: jsonb_array_insert") => {
                out.skips.push("sqlite 3.50.4 lacks json_array_insert (newer SQLite; Turso has it)".into());
                continue;
            },
            (Ok(s), Ok(r)) => (s, r),
            (s, r) => {
                let (signature, exact) = match (&s, &r) {
                    (Ok(_), Err(e)) => (format!("query-error: {}", normalize(e)), e.clone()),
                    (Err(e), _) => (format!("query-accepted: {}", normalize(e)), e.clone()),
                    _ => unreachable!(),
                };
                let jsonb = match &r {
                    Err(_) => jsonb.get_or_insert_with(|| jsonb_notes(&sdb)).as_str(),
                    Ok(_) => "",
                };
                out.findings.push(Finding {
                    signature,
                    detail: format!("{sql};\nSQLite: {}\nTurso: {}\n{note}{jsonb}", show(&s), show(&r)),
                    exact,
                    query: Some(qi),
                    turso: Some(r.clone()),
                    ordered: q.total_order(&schema).is_some(),
                });
                continue;
            },
        };
        let (mut s, mut r) = (s, r);
        jsonb_as_json(&sdb, &mut s);
        jsonb_as_json(&sdb, &mut r);
        out.compared += 1;
        let ordered = q.total_order(&schema).is_some();
        let (a, b) = if ordered { (s.clone(), r.clone()) } else { (sorted(&s), sorted(&r)) };
        if a == b {
            continue;
        }
        // SQLite's ANALYZE made sqlite_stat4, Turso's didn't
        // ([`sqlite_without_stat4`]): equal without it.
        let turso_stat4 = "SELECT count(*) FROM sqlite_schema WHERE name = 'sqlite_stat4'";
        if matches!(turso_query(&t, turso_stat4).as_deref(), Ok([row]) if row.first() == Some(&Cell::Int(0))) {
            if let Some(Ok(mut s4)) = sqlite_without_stat4(&sdb, &sql) {
                jsonb_as_json(&sdb, &mut s4);
                if (if ordered { s4 } else { sorted(&s4) }) == b {
                    out.skips.push("sqlite_stat4 exists in SQLite only (its STAT4 build; turso has no stat4)".into());
                    continue;
                }
            }
        }
        if lsql.contains("json") && json_digits(&a) == json_digits(&b) {
            out.skips.push("known: JSON reals printed with 17 digits, SQLite 3.50.4 15 (turso #8568)".into());
            continue;
        }
        // Documented: Turso substitutes U+FFFD for invalid UTF-8 (COMPAT.md).
        if has_fffd(&a) || has_fffd(&b) {
            out.skips.push("documented: invalid UTF-8 read as U+FFFD by turso".into());
            continue;
        }
        // Equal once a whole-number REAL counts as its integer, and -0.0 as 0.
        let (ca, cb) = (canonical(&a), canonical(&b));
        let (ca, cb) = if ordered { (ca, cb) } else { (sorted(&ca), sorted(&cb)) };
        if lsql.contains("||") && ca == text_for_blobs(&cb, &ca) {
            out.skips.push("known turso #6089: blob || blob is a blob".into());
            continue;
        }
        if ca == cb {
            let skip = if negative_zero_cells(&b) {
                "known turso #9889: -0.0 in a REAL column keeps its sign"
            } else if lsql.contains(" join ") {
                "known turso #9881: whole-number REAL as an integer through a join"
            } else if lsql.contains("distinct") || lsql.contains("group by") || lsql.contains(" union ") || lsql.contains(" intersect ") || lsql.contains(" except ") {
                "not a bug: which of two equal values (2 vs 2.0) DISTINCT/GROUP BY/compound keeps"
            } else {
                "rows equal up to 2 vs 2.0 (not attributed)"
            };
            out.skips.push(skip.into());
            continue;
        }
        let kind = if real_close(&a, &b) {
            "rows-differ (reals within 1e-12)"
        } else if a.len() != b.len() {
            "rows-differ (row count)"
        } else if order_dependent(&sql) {
            "rows-differ (order-dependent aggregate)"
        } else {
            "rows-differ"
        };
        let mode = if ordered { "ordered" } else { "as multisets" };
        out.findings.push(Finding {
            signature: format!("{kind} {}", features(&sql)),
            detail: format!("{sql};\ncompared {mode}\nSQLite: {}\nTurso: {}\n{note}", show(&Ok(s)), show(&Ok(r.clone()))),
            exact: String::new(),
            query: Some(qi),
            turso: Some(Ok(r)),
            ordered,
        });
    }
    out
}

/// Notes on the state after the statements, for rules on queries whose
/// output alone doesn't show the cause (counts, filters).
fn statement_notes(sdb: &Connection, t: &Turso, case: &Case, wr_ever: &[String]) -> String {
    let mut n = String::new();
    let stat1 = "SELECT count(*) FROM sqlite_schema WHERE name = 'sqlite_stat1'";
    let s_has = sdb.query_row(stat1, [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    let t_has = matches!(turso_query(t, stat1).as_deref(), Ok([row]) if row.first() == Some(&Cell::Int(1)));
    if s_has && !t_has {
        n.push_str("note: sqlite_stat1 exists in SQLite only (turso #10355 shape)\n");
    }
    let dropped_wr_column = case.statements.iter().map(|s| s.to_string().to_lowercase()).any(|s| {
        s.trim_start().starts_with("alter table") && s.contains(" drop ") && wr_ever.iter().any(|w| mentions(&s, w))
    });
    if dropped_wr_column {
        n.push_str("note: a column of a WITHOUT ROWID table was dropped (turso #9071 shape)\n");
    }
    let has_view = sdb.query_row("SELECT count(*) FROM sqlite_schema WHERE type = 'view'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    let altered = case.statements.iter().map(|s| s.to_string().to_lowercase()).any(|s| s.contains("drop table") || s.contains("alter table"));
    if has_view && altered {
        n.push_str("note: a view exists and a table was altered or dropped (turso #10397 shape)\n");
    }
    let typed = "SELECT count(*) FROM sqlite_schema s, pragma_table_info(s.name) p WHERE s.type = 'table' AND p.type LIKE '%(%'";
    if sdb.query_row(typed, [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0 {
        n.push_str("note: a declared type has parameters (turso #5762 shape)\n");
    }
    n
}

/// COMPAT.md: WITHOUT ROWID tables take plain INSERTs only (UPSERT is
/// rejected) and can't be a foreign key's parent.
fn documented_without_rowid(e: &str) -> bool {
    e.contains("ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint") || e.contains("foreign key mismatch")
}

/// `sql` (a statement, or an error naming tables) names one of `tables`.
fn writes_without_rowid(tables: &[String], sql: &str) -> bool {
    tables.iter().any(|t| mentions(sql, t))
}

/// The WITHOUT ROWID tables the case has at any point, lowercased: those in
/// the file, in SQLite after the statements, those any statement creates
/// (a later DROP or RENAME hides them from the others), and the names
/// `ALTER TABLE x RENAME TO y` gives one of them (the generator's t_swap
/// triple; inside SAVEPOINT .. ROLLBACK, SQLite's final schema shows the
/// old names again).
fn without_rowid_ever(db: &Connection, case: &Case) -> Vec<String> {
    let mut t: Vec<String> = without_rowid_tables(db).iter().map(|n| n.to_lowercase()).collect();
    // An input can start from an empty file (no header to read).
    if let Some(Ok(file)) = (case.file.0.len() >= 100).then(|| deserialized(&case.file.0)) {
        t.extend(without_rowid_tables(&file).iter().map(|n| n.to_lowercase()));
    }
    without_rowid_in_statements(t, &statement_texts(case))
}

/// `known` (WITHOUT ROWID tables, lowercased), the WITHOUT ROWID tables the
/// statements create, and the names a RENAME TO gives one of these, in
/// statement order.
fn without_rowid_in_statements(known: Vec<String>, stmts: &[String]) -> Vec<String> {
    let mut t = known;
    for s in stmts.iter().flat_map(|s| split_statements(s)) {
        let l = s.trim().to_lowercase();
        if l.contains("without rowid") {
            t.extend(created_name(&s));
        }
        let w: Vec<&str> = l.split_whitespace().collect();
        if let ["alter", "table", from, "rename", "to", to, ..] = w.as_slice() {
            if t.contains(&bare_name(from)) {
                t.push(bare_name(to));
            }
        }
    }
    t
}

/// `statement K: ...` from [`open_sqlite`]: K.
fn statement_index(e: &str) -> Option<usize> {
    e.strip_prefix("statement ")?.split(':').next()?.parse().ok()
}

/// The object a CREATE statement names (or the schema an ATTACH does),
/// lowercased, without its schema.
fn created_name(sql: &str) -> Option<String> {
    let words: Vec<String> = sql.split(|c: char| c.is_whitespace() || c == '(').filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect();
    if words.first().map(String::as_str) == Some("attach") {
        let at = words.iter().rposition(|w| w == "as")?;
        return words.get(at + 1).map(|w| w.trim_matches(|c| c == '"' || c == ';').to_string());
    }
    if words.first().map(String::as_str) != Some("create") {
        return None;
    }
    let at = words.iter().position(|w| ["table", "index", "view", "trigger"].contains(&w.as_str()))?;
    let mut rest = words[at + 1..].iter().skip_while(|w| ["if", "not", "exists"].contains(&w.as_str()));
    let name = rest.next()?;
    let name = name.rsplit('.').next()?.trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']');
    (!name.is_empty()).then(|| name.to_string())
}

/// `sql` names `name` as a whole word (case-insensitive).
fn mentions(sql: &str, name: &str) -> bool {
    let l = sql.to_lowercase();
    l.match_indices(name).any(|(i, _)| {
        let word = |c: char| c.is_alphanumeric() || c == '_';
        !l[..i].ends_with(word) && !l[i + name.len()..].starts_with(word)
    })
}

/// A statement SQLite 3.50.4 does not parse: `ALTER TABLE .. ALTER COLUMN
/// .. SET|DROP NOT NULL`, `ADD CONSTRAINT .. CHECK` and `DROP CONSTRAINT`
/// are SQLite 3.53's. They never compare (a syntax error before Turso
/// runs), and each one cost a retry and left the statements after it, drawn
/// for the model it would have made, to fail too.
fn newer_than_3_50_4(s: &Statement) -> bool {
    matches!(s, Statement::NotNull { .. } | Statement::AddCheck { .. } | Statement::DropCheck { .. } | Statement::DropConstraint { .. })
}

/// The input's statements from `from` on repaired one after another against
/// the model the ones before them leave (the generator's [`cases::repair`],
/// as its shrinker settles a smaller case), and its queries against the new
/// final schema: what a dropped statement made, everything naming it goes
/// or is rewritten, whatever the name (a table made in `aux` once ATTACH is
/// dropped, say). The model starts from `base`, the file as the engines get
/// it ([`model_base`]), and the input takes that file (the engines get the
/// same file either way, [`prepared`]). `false`, the input unchanged, when
/// that file's model does not read back or the model panics.
fn settle(input: &mut Input, from: usize, base: &SqliteFile) -> bool {
    let go = || -> Option<Input> {
        let mut m = cases::model_of(base)?;
        let (keep, rest) = input.0.statements.split_at(from.min(input.0.statements.len()));
        for s in keep {
            semantics::apply(&mut m, s);
        }
        let mut statements = keep.to_vec();
        for s in rest {
            if let Some(s) = cases::repair(&m, s) {
                semantics::apply(&mut m, &s);
                statements.push(s);
            }
        }
        // The input takes the file its statements are settled against: the
        // generator's shrinker models the input's own file, and statements
        // settled against another (`base` lacks the WITHOUT ROWID indexes)
        // made its write model panic ("a valid write") there.
        let case = Case { file: base.clone(), statements };
        let queries = repair_queries(&final_schema(&case), &input.1);
        Some((case, queries))
    };
    match panic::catch_unwind(AssertUnwindSafe(go)) {
        Ok(Some(settled)) => {
            *input = settled;
            true
        },
        _ => false,
    }
}

/// The generator's schema of the case after its statements, `None` when its
/// model does not read the file back or panics. It also sets what the
/// queries render against (the generator keeps the database's tables and
/// virtual tables per thread, set by `Schema::of`): a query rendered after
/// another case's schema was set can come out different (a CTE over a
/// virtual table as `x1 AS ()`), so the runner sets it before running or
/// writing an input's queries.
fn case_schema(case: &Case) -> Option<Schema> {
    panic::catch_unwind(AssertUnwindSafe(|| final_model(case).map(|m| Schema::of(&m)))).ok().flatten()
}

/// The case's file without the indexes [`drop_without_rowid_indexes`]
/// drops: what the statements run on, for [`settle`]'s model, so that the
/// statements naming those indexes go too (the rest of [`prepared`] keeps
/// every name). The case's own file when the generator's reader does not
/// read that one back.
fn model_base(case: &Case) -> SqliteFile {
    match drop_without_rowid_indexes(case) {
        Ok(c) if c.file != case.file && cases::model_of(&c.file).is_some() => c.file,
        _ => case.file.clone(),
    }
}

/// The SQLite build has what the generator draws (see the module docs).
fn sqlite_features() -> Result<(), String> {
    let db = Connection::open_in_memory().map_err(|e| e.to_string())?;
    for opt in ["ENABLE_STAT4", "ENABLE_GEOPOLY", "ENABLE_RTREE", "ENABLE_FTS5"] {
        let used: bool = db.query_row("SELECT sqlite_compileoption_used(?1)", [opt], |r| r.get(0)).map_err(|e| e.to_string())?;
        if !used {
            return Err(format!("SQLite is built without SQLITE_{opt}: build from this directory, where .cargo/config.toml sets LIBSQLITE3_FLAGS"));
        }
    }
    Ok(())
}

/// How many failing statements one input may drop before it is given up.
const RETRIES: usize = 8;

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// `run_input` on a database file of its own in `dir` (Turso keeps state
/// per path; a reused path would carry one input's state into the next),
/// a Turso panic caught as a finding.
fn check(input: &Input, dir: &Path, experimental: bool) -> Outcome {
    let path = dir.join(format!("db-{}.db", SERIAL.fetch_add(1, Ordering::Relaxed)));
    let mut o = check_at(input, &path, experimental);
    for ext in ["", "-wal", "-shm", "-log"] {
        let _ = fs::remove_file(format!("{}{ext}", path.display()));
    }
    let findings = std::mem::take(&mut o.findings);
    for f in findings {
        match trunk_check(input, &f, dir) {
            Trunk::Agrees => {
                o.trunk_checks += 1;
                o.skips.push("sqlite 3.50.4 quirk: sqlite trunk agrees with turso".into());
            },
            Trunk::Failed(e) => {
                o.trunk_checks += 1;
                o.trunk_failures.push(normalize(&e));
                o.findings.push(f);
            },
            Trunk::Differs => {
                o.trunk_checks += 1;
                o.findings.push(f);
            },
            Trunk::NotChecked => o.findings.push(f),
        }
    }
    o
}

fn check_at(input: &Input, path: &Path, experimental: bool) -> Outcome {
    match panic::catch_unwind(AssertUnwindSafe(|| run_input(input, path, experimental))) {
        Ok(o) => o,
        Err(p) => {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "?".into());
            let first = msg.lines().next().unwrap_or("").to_string();
            let step = STEP.with(|s| s.borrow().clone());
            Outcome { findings: vec![Finding { signature: format!("panic: {}", normalize(&first)), detail: format!("{step};\nTurso panicked: {msg}"), exact: first, query: None, ..Finding::default() }], ..Outcome::default() }
        },
    }
}

/// The message of a caught panic, and where it was raised (from
/// [`PANIC_AT`], which the runner's panic hook sets).
fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "?".into());
    let at = PANIC_AT.with(|a| a.borrow().clone());
    if at.is_empty() {
        msg
    } else {
        format!("{msg} (at {at})")
    }
}

/// What [`shrink`] found.
struct Shrunk {
    input: Input,
    /// The smallest input's finding; `None` when the input did not give the
    /// finding again ([`Shrunk::note`] says what it gave).
    finding: Option<Finding>,
    steps: usize,
    note: String,
}

/// The smallest input the generator's shrinker finds that still gives a
/// finding of `signature`, within a minute. A panic in the generator's
/// shrinker ends the shrinking with the smallest input so far.
fn shrink(input: &Input, target: &Finding, path: &Path, experimental: bool) -> Shrunk {
    // Same signature and message, and the same known issue (or none): a
    // smaller input that lands on another, already filed bug is not this one.
    let sql_of = |i: &Input, f: &Finding| f.query.and_then(|k| i.1.get(k)).map(|q| q.sql()).unwrap_or_else(|| f.detail.clone());
    let _ = case_schema(&input.0);
    let target_known = known(target, &sql_of(input, target), &statement_texts(&input.0));
    let same_in = |c: &Input, f: &Finding| f.signature == target.signature && f.exact == target.exact && known(f, &sql_of(c, f), &statement_texts(&c.0)) == target_known;
    let gen = case_queries();
    let mut best = input.clone();
    let mut why = None;
    let mut steps = 0;
    let mut note = String::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        let mut next = None;
        let round = panic::catch_unwind(AssertUnwindSafe(|| {
            gen.shrink(&best, &mut |c| {
                if start.elapsed() >= Duration::from_secs(60) {
                    return ControlFlow::Break(());
                }
                if let Some(f) = check(&c, path, experimental).findings.into_iter().find(|f| same_in(&c, f)) {
                    next = Some((c, f));
                    return ControlFlow::Break(());
                }
                ControlFlow::Continue(())
            })
        }));
        if let Err(p) = round {
            note = format!("the generator's shrinker panicked after {steps} steps: {}\n", panic_message(&*p));
            break;
        }
        match next {
            Some((c, f)) => {
                best = c;
                why = Some(f);
                steps += 1;
            },
            None => break,
        }
    }
    if why.is_none() {
        // No smaller input kept the finding: run the input itself again (its
        // queries rendered against its own schema, [`case_schema`]).
        let again = check(&best, path, experimental);
        why = again.findings.iter().find(|f| same_in(&best, f)).cloned();
        if why.is_none() {
            let gave: Vec<String> = again.findings.iter().map(|f| f.signature.clone()).chain(again.skips.iter().map(|s| format!("skip: {s}"))).collect();
            let _ = writeln!(
                note,
                "the input did not give the finding again when rerun (gave: {}): the engines' results on it are not deterministic (a time budget, randomness), or the finding depends on state from before this input",
                if gave.is_empty() { "nothing".to_string() } else { gave.join("; ") }
            );
        }
    }
    let _ = case_schema(&best.0);
    Shrunk { input: best, finding: why, steps, note }
}

// ------------------------------------------------------------ the runner

#[derive(Default)]
struct Stats {
    inputs: u64,
    /// Runs of an input again without a statement that failed.
    retries: u64,
    queries: u64,
    compared: u64,
    skips: BTreeMap<String, u64>,
    findings: BTreeMap<String, u64>,
    /// Findings SQLite trunk was asked about, and the helper's failures.
    trunk_checks: u64,
    trunk_failures: BTreeMap<String, u64>,
}

fn slug(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    t.chars().take(80).collect()
}

/// The case's statements as SQL text, one entry each (what [`known`] reads).
fn statement_texts(case: &Case) -> Vec<String> {
    case.statements.iter().map(|s| s.to_string()).collect()
}

fn script(case: &Case) -> String {
    case.statements.iter().map(|s| format!("{s};\n")).collect()
}

/// The queries, up to and including `upto` when given, one per line.
fn queries_sql(qs: &[Query], upto: Option<usize>) -> String {
    let n = upto.map_or(qs.len(), |k| (k + 1).min(qs.len()));
    qs[..n].iter().map(|q| format!("{};\n", q.sql())).collect()
}

/// A note when the case's file has an index on a WITHOUT ROWID table:
/// Turso cannot create one, so a reproduction needs a file SQLite made.
fn without_rowid_index_note(case: &Case) -> String {
    let Ok(c) = prepared(case) else { return String::new() };
    let Ok(db) = open_sqlite(&Case { file: c.file, statements: Vec::new() }) else { return String::new() };
    let n: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_schema i JOIN sqlite_schema t ON i.tbl_name = t.name \
             WHERE i.type = 'index' AND t.type = 'table' AND upper(t.sql) LIKE '%WITHOUT ROWID%'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if n > 0 {
        format!("note: the file has {n} index(es) on WITHOUT ROWID tables, which Turso cannot create; a plain-SQL reproduction needs SQLite to make the file\n")
    } else {
        String::new()
    }
}

/// Write the example (and its shrunk form) under `out/findings/<slug>/`.
fn record(out: &Path, input: &Input, f: &Finding, path: &Path, experimental: bool) {
    // The queries render against the input's own schema ([`case_schema`]),
    // not the one an earlier finding's shrinking left.
    let _ = case_schema(&input.0);
    let sql = f.query.and_then(|k| input.1.get(k)).map(|q| q.sql()).unwrap_or_else(|| f.detail.clone());
    if let Some(issue) = known(f, &sql, &statement_texts(&input.0)) {
        let ex = out.join("findings").join("known").join(issue);
        let _ = fs::create_dir_all(out.join("findings").join("known"));
        if fs::create_dir(&ex).is_err() {
            return;
        }
        let _ = fs::write(ex.join("signature.txt"), format!("{}\nmatched {}\n", f.signature, label(issue)));
        let _ = fs::write(ex.join("case.db"), prepared(&input.0).map(|c| c.file.0).unwrap_or_else(|_| input.0.file.0.clone()));
        let _ = fs::write(ex.join("replay.sql"), format!("{}{}", script(&input.0), queries_sql(&input.1, f.query)));
        let _ = fs::write(ex.join("finding.txt"), format!("{}\n", f.detail));
        return;
    }
    let dir = out.join("findings").join(slug(&f.signature));
    let _ = fs::create_dir_all(&dir);
    // The first free slot, claimed by creating it: two workers recording
    // the same signature at once wrote into one directory.
    let Some(ex) = (0..EXAMPLES).map(|n| dir.join(format!("{n}"))).find(|ex| fs::create_dir(ex).is_ok()) else { return };
    let _ = fs::write(ex.join("signature.txt"), format!("{}\n", f.signature));
    let _ = fs::write(ex.join("case.db"), prepared(&input.0).map(|c| c.file.0).unwrap_or_else(|_| input.0.file.0.clone()));
    let _ = fs::write(ex.join("statements.sql"), script(&input.0));
    let note = without_rowid_index_note(&input.0);
    let _ = fs::write(ex.join("finding.txt"), format!("{}\n{note}", f.detail));
    let _ = fs::write(ex.join("queries.sql"), queries_sql(&input.1, None));
    let _ = fs::write(ex.join("replay.sql"), format!("{}{}", script(&input.0), queries_sql(&input.1, f.query)));
    // The shrinking can't lose the example: a panic in it keeps the
    // unshrunk files, with the panic's message.
    let small = match panic::catch_unwind(AssertUnwindSafe(|| shrink(input, f, path, experimental))) {
        Ok(s) => s,
        Err(p) => {
            let _ = fs::write(ex.join("shrink-panicked.txt"), format!("shrinking panicked: {}\nuse the unshrunk files\n", panic_message(&*p)));
            return;
        },
    };
    if !small.note.is_empty() {
        let _ = fs::write(ex.join("shrink-note.txt"), &small.note);
    }
    let Some(why) = small.finding else { return };
    let small_input = &small.input;
    let _ = fs::write(ex.join("shrunk-case.db"), prepared(&small_input.0).map(|c| c.file.0).unwrap_or_else(|_| small_input.0.file.0.clone()));
    let _ = fs::write(ex.join("shrunk-statements.sql"), script(&small_input.0));
    let _ = fs::write(ex.join("shrunk-queries.sql"), queries_sql(&small_input.1, None));
    let _ = fs::write(ex.join("shrunk-replay.sql"), format!("{}{}", script(&small_input.0), queries_sql(&small_input.1, why.query)));
    let note = without_rowid_index_note(&small_input.0);
    let _ = fs::write(ex.join("shrunk-finding.txt"), format!("{}\n{note}shrink steps: {}\n", why.detail, small.steps));
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn report(stats: &Stats, elapsed: Duration) -> String {
    let mut s = format!(
        "{:.0}s: {} inputs ({:.1}/s), {} retries, {} queries, {} compared\n",
        elapsed.as_secs_f64(),
        stats.inputs,
        stats.inputs as f64 / elapsed.as_secs_f64().max(1.0),
        stats.retries,
        stats.queries,
        stats.compared
    );
    let agreed = stats.skips.get("sqlite 3.50.4 quirk: sqlite trunk agrees with turso").copied().unwrap_or(0);
    let failed: u64 = stats.trunk_failures.values().sum();
    match TRUNK_ORACLE.get() {
        Some(Some(_)) => {
            let _ = writeln!(s, "trunk oracle: {} findings checked, {agreed} skipped (trunk agrees with turso), {failed} helper failures", stats.trunk_checks);
            for (k, v) in &stats.trunk_failures {
                let _ = writeln!(s, "  {v:7} trunk oracle failed: {k}");
            }
        },
        _ => s.push_str("trunk oracle: off (--no-trunk-oracle)\n"),
    }
    let mut skips: Vec<_> = stats.skips.iter().collect();
    skips.sort_by(|a, b| b.1.cmp(a.1));
    s.push_str("skips:\n");
    for (k, v) in skips.iter().take(15) {
        let _ = writeln!(s, "  {v:7} {k}");
    }
    let mut finds: Vec<_> = stats.findings.iter().collect();
    finds.sort_by(|a, b| b.1.cmp(a.1));
    s.push_str("findings:\n");
    for (k, v) in &finds {
        let _ = writeln!(s, "  {v:7} {k}");
    }
    s
}

/// `--replay DB SQL`: a copy of the database file DB opened by Turso and by
/// SQLite, each statement of the file SQL run on both, and what each gives
/// printed.
fn replay(db: &Path, sql: &Path, experimental: bool) {
    // The file as a run gives it to both engines ([`prepared`]); a shrunk
    // example's shrunk-case.db already is.
    let raw = fs::read(db).expect("database file");
    let bytes = prepared(&Case { file: SqliteFile(raw.clone()), statements: Vec::new() }).map(|c| c.file.0).unwrap_or(raw);
    let text = fs::read_to_string(sql).expect("SQL file");
    let path = std::env::temp_dir().join(format!("turso-diff-replay-{}.db", std::process::id()));
    for ext in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{ext}", path.display()));
    }
    fs::write(&path, &bytes).expect("copy");
    let mut b = turso::Builder::new_local(&path.to_string_lossy());
    if experimental {
        b = b.experimental_without_rowid(true).experimental_generated_columns(true).experimental_attach(true);
    }
    let tdb = pollster::block_on(b.build()).expect("turso open");
    let t = Turso { conn: tdb.connect().expect("turso connect"), _db: tdb };
    let mut sdb = sqlite_memory().unwrap();
    if !bytes.is_empty() {
        let mut b2 = bytes.clone();
        for at in [18, 19] {
            if b2[at] == 2 {
                b2[at] = 1;
            }
        }
        sdb.deserialize_read_exact(rusqlite::MAIN_DB, &b2[..], b2.len(), false).unwrap();
    }
    for one in split_statements(&text) {
        if one.trim().is_empty() {
            continue;
        }
        println!("{}", one.trim());
        println!("  SQLite: {}", show(&sqlite_query(&sdb, &one)).replace('\n', "\n  "));
        println!("  Turso:  {}", show(&turso_query(&t, &one)).replace('\n', "\n  "));
    }
}

/// The rows of a [`show`] output (its first block), sorted.
fn shown_lines(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.lines().take_while(|l| l.starts_with("  ") || l.ends_with(" rows")).map(str::to_string).collect();
    v.sort();
    v
}

/// Whether a [`show`] output (Turso's side of a finding.txt) has as many
/// rows as `rows` and each row it shows (the first 20) is one of `rows`,
/// counted with multiplicity: for up to 20 rows, the same multiset.
fn shown_within(rows: &Rows, shown: &str) -> bool {
    let mut lines = shown.lines();
    if lines.next() != Some(&format!("{} rows", rows.len())) {
        return false;
    }
    let mut pool: Vec<String> = rows.iter().map(|r| show(&Ok(vec![r.clone()])).lines().nth(1).unwrap_or("").to_string()).collect();
    for l in lines.take_while(|l| l.starts_with("  ")) {
        if l == "  ..." {
            continue;
        }
        match pool.iter().position(|p| p == l) {
            Some(i) => {
                pool.swap_remove(i);
            },
            None => return false,
        }
    }
    true
}

/// `--retriage RUN_DIR`: every example a run kept under `findings/`
/// classified again from its files, as the run would classify it now: the
/// known-issue rules ([`known`], with [`jsonb_notes`] made from `case.db`
/// and the statements when the finding lacks them), the documented
/// WITHOUT ROWID skip, and SQLite trunk ([`trunk_check`]; Turso's side read
/// back from `finding.txt`). Prints one line per example (old and new
/// label) and the totals by new label.
fn retriage(run: &Path) {
    let helper = trunk_oracle_path();
    let tmp = std::env::temp_dir().join(format!("turso-diff-retriage-{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);
    let trunk = match trunk_oracle_version(&helper, &tmp) {
        Ok(v) => {
            println!("trunk oracle: {} ({v})", helper.display());
            Some(helper)
        },
        Err(e) => {
            println!("trunk oracle: not used ({e}; build it with: {TRUNK_ORACLE_BUILD})");
            None
        },
    };
    let mut examples: Vec<(PathBuf, String)> = Vec::new();
    let Ok(dirs) = fs::read_dir(run.join("findings")) else {
        eprintln!("no findings/ in {}", run.display());
        std::process::exit(2);
    };
    let mut dirs: Vec<PathBuf> = dirs.filter_map(Result::ok).map(|d| d.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for d in dirs {
        let name = d.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if name == "hang" {
            continue;
        }
        let mut subs: Vec<PathBuf> = fs::read_dir(&d).map(|r| r.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
        subs.sort();
        for e in subs {
            let old = if name == "known" { e.file_name().unwrap_or_default().to_string_lossy().into_owned() } else { "-".into() };
            examples.push((e, old));
        }
    }
    let mut totals: BTreeMap<String, u64> = BTreeMap::new();
    for (ex, old) in &examples {
        let read = |f: &str| fs::read_to_string(ex.join(f)).ok();
        let (Some(sig), Some(detail)) = (read("signature.txt"), read("finding.txt")) else { continue };
        let sig = sig.lines().next().unwrap_or("").to_string();
        // statements.sql (a kept example) holds the statements only;
        // replay.sql (known/) ends with the query.
        let from_replay = read("statements.sql").is_none();
        let stmts: Vec<String> = script_entries(&read("statements.sql").or_else(|| read("replay.sql")).unwrap_or_default());
        let is_query = sig.starts_with("query-") || sig.starts_with("rows-differ");
        let sql = if is_query { detail.split(";\nSQLite:").next().and_then(|s| s.split(";\ncompared").next()).unwrap_or(&detail).to_string() } else { detail.clone() };
        let line_after = |p: &str| detail.lines().find_map(|l| l.strip_prefix(p)).unwrap_or("").to_string();
        let exact = if sig.starts_with("query-accepted") { line_after("SQLite: error: ") } else { line_after("Turso: error: ") };
        // The state after the statements, in SQLite, for the notes and the
        // WITHOUT ROWID tables.
        let db = fs::read(ex.join("case.db")).ok().and_then(|b| if b.is_empty() { sqlite_memory().ok() } else { deserialized(&b).ok() });
        if let Some(db) = &db {
            for st in stmts.iter().take(if is_query && from_replay { stmts.len().saturating_sub(1) } else { stmts.len() }) {
                sqlite_budget();
                let _ = db.execute_batch(st);
            }
        }
        let mut detail = detail.clone();
        if sig.starts_with("query-error") && !detail.contains("candidate jsonb-text") && !detail.contains("draft json-blob") {
            if let Some(db) = &db {
                detail.push_str(&jsonb_notes(db));
            }
        }
        let f = Finding { signature: sig.clone(), detail: detail.clone(), exact: exact.clone(), ..Finding::default() };
        let mut new = known(&f, &sql, &stmts).map(label);
        if sig.starts_with("statement-error") && documented_without_rowid(&exact) {
            let mut wr: Vec<String> = db.as_ref().map(|d| without_rowid_tables(d).iter().map(|n| n.to_lowercase()).collect()).unwrap_or_default();
            if let Ok(b) = fs::read(ex.join("case.db")) {
                if let Some(Ok(file)) = (b.len() >= 100).then(|| deserialized(&b)) {
                    wr.extend(without_rowid_tables(&file).iter().map(|n| n.to_lowercase()));
                }
            }
            let wr = without_rowid_in_statements(wr, &stmts);
            if writes_without_rowid(&wr, &sql) || writes_without_rowid(&wr, &exact) {
                new = Some("skip: documented (WITHOUT ROWID)".into());
            }
        }
        if new.is_none() && sig.starts_with("rows-differ") {
            // Only where the table changes SQLite's rows, and Turso's rows
            // (finding.txt shows at most 20) are SQLite's without it.
            if let (Some(d), Some(Ok(mut rows))) = (&db, db.as_ref().and_then(|d| sqlite_without_stat4(d, &sql))) {
                if let Ok(mut with) = sqlite_query(d, &sql) {
                    jsonb_as_json(d, &mut rows);
                    jsonb_as_json(d, &mut with);
                    let turso = detail.split("\nTurso: ").nth(1).unwrap_or("");
                    if sorted(&with) != sorted(&rows) && shown_within(&rows, turso) {
                        new = Some("skip: sqlite_stat4 exists in SQLite only".into());
                    }
                }
            }
        }
        if new.is_none() && is_query {
            if let (Some(helper), Ok(b), Some(replay)) = (&trunk, fs::read(ex.join("case.db")), read("replay.sql")) {
                if let Ok(t) = run_trunk_oracle(helper, &b, &replay, &tmp) {
                    let turso = detail.split("\nTurso: ").nth(1).unwrap_or("");
                    let shown = match t.lines().next() {
                        Some(h) if h.starts_with("E:") => Some(Err(h[2..].to_string())),
                        Some("OK") => {
                            let rows: Rows = t.lines().skip(1).map(|l| l.split('\t').filter(|c| !c.is_empty()).map(|c| match c.split_once(':') {
                                Some(("I", v)) => Cell::Int(v.parse().unwrap_or(0)),
                                Some(("R", v)) => Cell::Real(f64::from_bits(u64::from_str_radix(v, 16).unwrap_or(0))),
                                Some(("T", v)) => Cell::Text(String::from_utf8_lossy(&unhex(v)).into_owned()),
                                Some(("B", v)) => Cell::Blob(unhex(v)),
                                Some(("J", v)) => Cell::Json(String::from_utf8_lossy(&unhex(v)).into_owned()),
                                _ => Cell::Null,
                            }).collect()).collect();
                            Some(Ok(rows))
                        },
                        _ => None,
                    };
                    let agrees = match shown {
                        Some(Err(_)) => turso.starts_with("error: "),
                        Some(Ok(rows)) => shown_lines(&show(&Ok(rows))) == shown_lines(turso),
                        None => false,
                    };
                    if agrees {
                        new = Some("skip: sqlite trunk agrees with turso".into());
                    }
                }
            }
        }
        let new = new.unwrap_or_else(|| "-".into());
        let rel = ex.strip_prefix(run.join("findings")).unwrap_or(ex).display().to_string();
        let old_label = if old == "-" { "-".to_string() } else { label(old) };
        println!("{rel}\t{old_label}\t->\t{new}");
        *totals.entry(new).or_default() += 1;
    }
    let _ = fs::remove_dir_all(&tmp);
    println!("\n{} examples:", examples.len());
    let mut t: Vec<_> = totals.into_iter().collect();
    t.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (k, v) in t {
        println!("  {v:4} {k}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--retriage") {
        retriage(Path::new(&args[i + 1]));
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--replay") {
        replay(Path::new(&args[i + 1]), Path::new(&args[i + 2]), args.iter().any(|a| a == "--experimental"));
        return;
    }
    let seed: u64 = arg(&args, "--seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let minutes: f64 = arg(&args, "--minutes").and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let jobs: u64 = arg(&args, "--jobs").and_then(|s| s.parse().ok()).unwrap_or(4);
    let Some(out) = arg(&args, "--out").map(PathBuf::from) else {
        eprintln!("usage: turso-diff --out DIR [--seed S] [--minutes M] [--jobs J] [--experimental] [--no-trunk-oracle]\n       turso-diff --replay DB SQLFILE [--experimental]\n       turso-diff --retriage RUN_DIR");
        std::process::exit(2);
    };
    let experimental = args.iter().any(|a| a == "--experimental");
    if let Err(e) = sqlite_features() {
        eprintln!("{e}");
        std::process::exit(2);
    }
    fs::create_dir_all(out.join("tmp")).expect("output directory");
    // SQLite trunk, asked about each finding before it is kept: a missing or
    // broken helper would silently keep every SQLite 3.50.4 quirk.
    if args.iter().any(|a| a == "--no-trunk-oracle") {
        println!("trunk oracle: off (--no-trunk-oracle); findings where SQLite 3.50.4 is wrong and trunk agrees with Turso are kept");
        let _ = TRUNK_ORACLE.set(None);
    } else {
        let helper = trunk_oracle_path();
        match trunk_oracle_version(&helper, &out.join("tmp")) {
            Ok(v) => {
                println!("trunk oracle: {} ({v})", helper.display());
                let _ = fs::write(out.join("trunk-oracle.txt"), format!("{}\n{v}\n", helper.display()));
                let _ = TRUNK_ORACLE.set(Some(helper));
            },
            Err(e) => {
                eprintln!("{e}.\nBuild it with:\n  {TRUNK_ORACLE_BUILD}\n(or set TURSO_DIFF_TRUNK_ORACLE to the helper's path, or pass --no-trunk-oracle to run without the trunk check)");
                std::process::exit(2);
            },
        }
    }
    // Turso panics are findings; keep their default messages off stderr,
    // and where each was raised for [`panic_message`].
    panic::set_hook(Box::new(|info| {
        let at = info.location().map(|l| l.to_string()).unwrap_or_default();
        PANIC_AT.with(|a| *a.borrow_mut() = at);
    }));

    let stats = Arc::new(Mutex::new(Stats::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + Duration::from_secs_f64(minutes * 60.0);
    let mut beats = Vec::new();
    for w in 0..jobs {
        let (stats, stop, out) = (stats.clone(), stop.clone(), out.clone());
        let beat = Arc::new(AtomicU64::new(0));
        beats.push(beat.clone());
        thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || {
                let gen = case_queries();
                let path = out.join("tmp").join(format!("w{w}"));
                let _ = fs::create_dir_all(&path);
                let start = Instant::now();
                let mut n = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let s = seed.wrapping_mul(1_000_003).wrapping_add(w * 1_000_000_007 + n);
                    let mut input = match panic::catch_unwind(AssertUnwindSafe(|| gen.generate(&mut Source::new(s, (n % 101) as usize)))) {
                        Ok(i) => i,
                        Err(_) => {
                            n += 1;
                            continue;
                        },
                    };
                    // COMPAT.md: PRAGMA optimize is unsupported, but Turso
                    // accepts it as a no-op while SQLite may ANALYZE.
                    // Same for reverse_unordered_selects (unsupported in COMPAT.md).
                    // And statements newer than 3.50.4 ([`newer_than_3_50_4`]),
                    // the rest settled against the model they leave.
                    let before = input.0.statements.len();
                    input.0.statements.retain(|s| {
                        let l = s.to_string().to_lowercase();
                        !newer_than_3_50_4(s) && !l.replace(' ', "").contains("pragmaoptimize") && !l.contains(".optimize") && !l.contains("reverse_unordered_selects")
                    });
                    let base = model_base(&input.0);
                    if input.0.statements.len() < before || base != input.0.file {
                        settle(&mut input, 0, &base);
                    }
                    // A statement either engine rejects ends the input; it is
                    // dropped (after its finding, if any, is recorded) and the
                    // rest runs again, so one known bug or unsupported feature
                    // doesn't hide the queries behind it.
                    for attempt in 0..=RETRIES {
                        // The input in flight, for a hang finding.
                        let _ = fs::write(out.join("tmp").join(format!("w{w}.inflight.sql")), format!("-- seed {s}\n{}\n-- queries\n{}", script(&input.0), input.1.iter().map(|q| format!("{};\n", q.sql())).collect::<String>()));
                        let _ = fs::write(out.join("tmp").join(format!("w{w}.inflight.db")), &input.0.file.0);
                        beat.store(start.elapsed().as_secs() + 1, Ordering::Relaxed);
                        let o = check(&input, &path, experimental);
                        beat.store(0, Ordering::Relaxed);
                        {
                            let mut st = stats.lock().unwrap();
                            st.inputs += (attempt == 0) as u64;
                            st.retries += (attempt > 0) as u64;
                            st.queries += o.queries;
                            st.compared += o.compared;
                            for k in &o.skips {
                                *st.skips.entry(k.clone()).or_default() += 1;
                            }
                            st.trunk_checks += o.trunk_checks;
                            for k in &o.trunk_failures {
                                *st.trunk_failures.entry(k.clone()).or_default() += 1;
                            }
                            for f in &o.findings {
                                let sql = f.query.and_then(|k| input.1.get(k)).map(|q| q.sql()).unwrap_or_else(|| f.detail.clone());
                                let key = match known(f, &sql, &statement_texts(&input.0)) {
                                    Some(issue) => format!("known {}: {}", label(issue), f.signature),
                                    None => f.signature.clone(),
                                };
                                *st.findings.entry(key).or_default() += 1;
                            }
                        }
                        for f in &o.findings {
                            beat.store(start.elapsed().as_secs() + 1 + 600, Ordering::Relaxed);
                            record(&out, &input, f, &path, experimental);
                            beat.store(0, Ordering::Relaxed);
                        }
                        match o.failed_statement {
                            Some(k) if k < input.0.statements.len() => {
                                let gone = input.0.statements.remove(k).to_string();
                                // Statements naming what it would have created
                                // would fail next, one retry each: the rest is
                                // settled against the model, or, when that
                                // fails, those naming its name are dropped.
                                if !settle(&mut input, k, &base) {
                                    if let Some(name) = created_name(&gone) {
                                        let mut rest = input.0.statements.split_off(k);
                                        rest.retain(|s| !mentions(&s.to_string(), &name));
                                        input.0.statements.extend(rest);
                                    }
                                }
                            },
                            _ => break,
                        }
                    }
                    n += 1;
                }
            })
            .expect("worker thread");
    }
    let start = Instant::now();
    let mut last = Instant::now();
    let mut hung = vec![false; jobs as usize];
    while Instant::now() < deadline && hung.iter().any(|h| !h) {
        thread::sleep(Duration::from_secs(1));
        for (w, b) in beats.iter().enumerate() {
            let since = b.load(Ordering::Relaxed);
            if !hung[w] && since > 0 && start.elapsed().as_secs() + 1 > since + HANG.as_secs() {
                hung[w] = true;
                let dir = out.join("findings").join("hang").join(format!("w{w}"));
                let _ = fs::create_dir_all(&dir);
                for f in ["inflight.sql", "inflight.db"] {
                    let _ = fs::copy(out.join("tmp").join(format!("w{w}.{f}")), dir.join(f));
                }
                stats.lock().unwrap().findings.entry("hang".into()).and_modify(|v| *v += 1).or_insert(1);
                eprintln!("worker {w} hung; its input is in {}", dir.display());
            }
        }
        if last.elapsed() >= Duration::from_secs(30) {
            last = Instant::now();
            let r = report(&stats.lock().unwrap(), start.elapsed());
            println!("{r}");
            let _ = fs::write(out.join("stats.txt"), &r);
        }
    }
    stop.store(true, Ordering::Relaxed);
    let r = report(&stats.lock().unwrap(), start.elapsed());
    println!("{r}");
    let _ = fs::write(out.join("stats.txt"), &r);
    // Hung workers cannot be joined; leave without waiting for them.
    std::process::exit(0);
}


/// The known-issue rules on findings of run 20261026-gen0b23 (its sweep,
/// `turso-diff/runs/20261026-gen0b23/sweep.md`, gives each one's cause):
/// the text each rule reads, cut to what it needs.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat4_dropped_for_the_rerun_only() {
        let db = sqlite_memory().unwrap();
        assert!(sqlite_without_stat4(&db, "SELECT 1").is_none());
        db.execute_batch("CREATE TABLE t(a); CREATE INDEX i ON t(a); INSERT INTO t VALUES (1), (2); ANALYZE; BEGIN; INSERT INTO t VALUES (3);").unwrap();
        let list = "SELECT name FROM pragma_table_list WHERE name LIKE 'sqlite_stat%' ORDER BY 1";
        let without = sqlite_without_stat4(&db, list).unwrap().unwrap();
        assert_eq!(without, vec![vec![Cell::Text("sqlite_stat1".into())]]);
        // Still there afterwards, and the open transaction is kept.
        assert_eq!(sqlite_query(&db, list).unwrap().len(), 2);
        assert!(!db.is_autocommit());
        assert_eq!(sqlite_query(&db, "SELECT count(*) FROM t").unwrap(), vec![vec![Cell::Int(3)]]);
        let shown = show(&Ok(without.clone()));
        assert!(shown_within(&without, &shown));
        assert!(!shown_within(&sqlite_query(&db, list).unwrap(), &shown));
    }

    fn finding(signature: &str, detail: &str, exact: &str) -> Finding {
        Finding { signature: signature.into(), detail: detail.into(), exact: exact.into(), ..Finding::default() }
    }

    fn strings(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    /// A statement error: its detail is also what the rules read as its SQL.
    fn statement(signature: &str, detail: &str, exact: &str, stmts: &[&str]) -> Option<&'static str> {
        known(&finding(signature, detail, exact), detail, &strings(stmts))
    }

    /// A query finding: detail is `{sql};\nSQLite: ..\nTurso: ..\n{notes}`.
    fn query(signature: &str, sql: &str, rest: &str, exact: &str, stmts: &[&str]) -> Option<&'static str> {
        known(&finding(signature, &format!("{sql};\n{rest}"), exact), sql, &strings(stmts))
    }

    const WR: &str = "note: the statement names a WITHOUT ROWID table\n";

    #[test]
    fn not_null_after_drop_column_on_without_rowid_is_9071() {
        // statement_error__NOT_NULL_constraint_failed/0: the column added back.
        let d = format!("statement 2: ALTER TABLE `t2` ADD COLUMN c5 ANY CONSTRAINT co5 COLLATE BINARY CONSTRAINT nn5 NOT NULL CONSTRAINT `df5` DEFAULT (-2);\nSQLite: ok\nTurso: error: NOT NULL constraint failed\n{WR}");
        let before = ["PRAGMA cache_size = 10", "ALTER TABLE T2 DROP COLUMN C4; ALTER TABLE t2 RENAME c5 TO \"c4\""];
        assert_eq!(statement("statement-error: NOT NULL constraint failed", &d, "NOT NULL constraint failed", &before), Some("9071"));
        // Without the DROP before it: not this.
        assert_eq!(statement("statement-error: NOT NULL constraint failed", &d, "NOT NULL constraint failed", &["PRAGMA cache_size = 10", "SELECT 1"]), None);
        // A DROP after the failing statement does not count.
        let after = ["PRAGMA cache_size = 10", "SELECT 1", "ALTER TABLE t2 DROP COLUMN c4"];
        assert_eq!(statement("statement-error: NOT NULL constraint failed", &d, "NOT NULL constraint failed", &after), None);
        // .._tN_cN/1 and /0: `INSERT .. SELECT * FROM` the table read back.
        let d = format!("statement 1: INSERT INTO t3 SELECT * FROM (SELECT * FROM t0);\nSQLite: ok\nTurso: error: NOT NULL constraint failed: t3.c4\n{WR}");
        let drop = ["ALTER TABLE `t0` DROP \"c4\"; ALTER TABLE t0 RENAME COLUMN `c5` TO c4"];
        assert_eq!(statement("statement-error: NOT NULL constraint failed: tN.cN", &d, "NOT NULL constraint failed: t3.c4", &drop), Some("9071"));
        let d = format!("statement 1: EXPLAIN INSERT INTO t2 SELECT * FROM t1 NOT INDEXED; INSERT INTO t2 SELECT * FROM t1 NOT INDEXED;\nSQLite: ok\nTurso: error: NOT NULL constraint failed: t2.c1\n{WR}");
        let drop = ["ALTER TABLE `t1` DROP COLUMN c1; ALTER TABLE t1 RENAME COLUMN \"C2\" TO c1; ALTER TABLE t1 RENAME C3 TO c2"];
        assert_eq!(statement("statement-error: NOT NULL constraint failed: tN.cN", &d, "NOT NULL constraint failed: t2.c1", &drop), Some("9071"));
    }

    #[test]
    fn min_int_default_is_10391_through_renames() {
        // cannot_store/1: the column moved to t4 by the t_swap triple, then renamed.
        let d = "statement 2: UPDATE t4 SET oid = (oid + 4611686018427387904), c3 = 'ж' WHERE c5 < -9223372036854775807 RETURNING c2, c1;\nSQLite: ok\nTurso: error: cannot store REAL value in INT column t4.c5 (19)\n";
        let stmts = [
            "ALTER TABLE t0 ADD COLUMN c6 INT CONSTRAINT \"df6\" DEFAULT -9223372036854775808",
            "ALTER TABLE t0 RENAME TO t_swap; ALTER TABLE t4 RENAME TO t0; ALTER TABLE t_swap RENAME TO t4; ALTER TABLE t4 RENAME COLUMN c6 TO \"c5\"",
        ];
        assert_eq!(statement("statement-error: cannot store REAL value in INT column tN.cN (N)", d, "cannot store REAL value in INT column t4.c5 (19)", &stmts), Some("10391"));
        assert_eq!(statement("statement-error: cannot store REAL value in INT column tN.cN (N)", d, "", &stmts[1..]), None);
        // rows_differ__json_distinct_/0: old rows read the default as a REAL.
        let rest = "compared as multisets\nSQLite: 1 rows\n  x''|5270415|-24.45777651576021|\"-9223372036854775808\"\nTurso: 1 rows\n  x''|5270415|-24.45777651576021|\"-9.22337203685478e+18\"\n";
        let stmts = ["ALTER TABLE t1 ADD c3 NVARCHAR(100) NOT NULL DEFAULT -9223372036854775808"];
        assert_eq!(query("rows-differ [json,distinct]", "SELECT DISTINCT * FROM main.t1 AS a0 WHERE (\"c3\" <= (x'' NOT GLOB json_quote('жa1')))", rest, "", &stmts), Some("10391"));
    }

    #[test]
    fn cte_only_in_the_left_operand_of_in_is_8655() {
        let rest = "SQLite: 0 rows\nTurso: error: Parse error: no such table: x0\n";
        let sig = "query-error: Parse error: no such table: xN";
        let exact = "Parse error: no such table: x0";
        // query_error__Parse_error__no_such_table__xN/1
        let q = "WITH x0 AS MATERIALIZED (SELECT c0 AS c0, nth_value(c1, 2) OVER (PARTITION BY c1 ORDER BY rowid ASC NULLS LAST RANGE BETWEEN CURRENT ROW AND 20 FOLLOWING) AS c1 FROM main.t1 AS a0) SELECT * FROM main.t1 AS a0 WHERE ((SELECT count(c1) FROM x0 b0 WHERE (c0 <> '2024-01-28 00:00')) IN ((SELECT count(DISTINCT id) FROM json_each(jsonb_array(0)))))";
        assert_eq!(query(sig, q, rest, exact, &[]), Some("8655"));
        // .._xN/2: a row value whose second element reads the CTE.
        let q = "WITH x0 AS MATERIALIZED (SELECT cid AS c0, name AS c1, type AS c2, \"notnull\" AS c3, dflt_value AS c4, pk AS c5, hidden AS c6 FROM pragma_table_xinfo('t1')) SELECT DISTINCT * FROM t3 AS a0 INNER JOIN t1 AS a1 USING (c0) WHERE (((a1.c0, a1.c0, (SELECT ((count(c5) IS NULL) IS NULL) FROM x0 b0 WHERE (a0.c2 IN (SELECT c0 FROM main.t2 AS d0)))) IN (SELECT c1, c0, c2 FROM main.t7 b0 WHERE (CAST(likely('%') AS INTEGER) / c2))) AND (a1.c0 = a1.c0))";
        assert_eq!(query(sig, q, rest, exact, &[]), Some("8655"));
        // .._xN/0: recursive, read in the right side of an IN that is itself
        // in the left operand of another.
        let q = "WITH RECURSIVE x0(c0) AS (SELECT 1 AS c0 UNION ALL SELECT c0 + 1 FROM x0 LIMIT 9) SELECT max(c0) FROM main.t4 a0 WHERE ((c2 IS 85.125) OR (((-23.0 IN (SELECT c1 FROM x0)) IN (SELECT c0 FROM t4))))";
        assert_eq!(query(sig, q, rest, exact, &[]), Some("8655"));
        // known/10354 (and 10355): read in a subquery of a JOIN's ON clause,
        // a query on pragma_index_xinfo: #9073, not #10354.
        let q = "WITH x0 AS NOT MATERIALIZED (SELECT v0.name AS c0, v1.cid AS c2 FROM pragma_index_list('t0') AS v0, pragma_index_xinfo(v0.name) AS v1) SELECT * FROM t0 AS a0 JOIN t1 AS a1 ON (a1.c0 IN (SELECT c2 FROM x0))";
        assert_eq!(query(sig, q, rest, exact, &[]), Some("9073"));
        let q = "WITH x1 AS MATERIALIZED (SELECT schema AS c0, name AS c1 FROM pragma_table_list) SELECT DISTINCT * FROM t3 AS a0 RIGHT JOIN t2 AS a1 ON ((SELECT count(*) FROM x1) > 0)";
        let rest = "SQLite: 1 rows\n  \"main\"|\"sqlite_stat1\"\nTurso: error: Parse error: no such table: x1\nnote: sqlite_stat1 exists in SQLite only (turso #10355 shape)\n";
        assert_eq!(query(sig, q, rest, "Parse error: no such table: x1", &[]), Some("9073"));
    }

    #[test]
    fn drop_column_with_references_is_9077() {
        let d = "statement 29: ALTER TABLE t1 DROP \"C1\";\nSQLite: ok\nTurso: error: Parse error: error in table t1 after drop column: unknown column \"C1\" in foreign key definition\n";
        let sig = "statement-error: Parse error: error in table TN after drop column: unknown column '?' in foreign key defini";
        assert_eq!(statement(sig, d, "Parse error: error in table t1 after drop column: unknown column \"C1\" in foreign key definition", &[]), Some("9077"));
    }

    #[test]
    fn real_column_against_big_exponent_text_is_9849() {
        // rows_differ__distinct_/0
        let q = "SELECT DISTINCT * FROM main.t1 AS a0 INNER JOIN t1 AS a1 ON (a1.c0 <> 6.772077710672306) WHERE ((a1.c0 NOT BETWEEN -9e999 AND '-8.506499711143285e260') AND (a1.c0 NOT BETWEEN 6.772077710672306 AND 6.772077710672306)) ORDER BY 1, 2 ASC NULLS LAST LIMIT 1 OFFSET 0";
        let rest = "compared ordered\nSQLite: 1 rows\n  -inf|-1.0\nTurso: 1 rows\n  -inf|-8.506499711143285e260\n";
        assert_eq!(query("rows-differ [distinct]", q, rest, "", &[]), Some("9849"));
        // rows_differ__row_count___plain_/0
        let q = "SELECT * FROM t0 AS a0 WHERE (c0 NOT BETWEEN '3.4695064670173946e126' AND (5770710625472121 * 1.0 * 4611686018427387904))";
        assert_eq!(query("rows-differ (row count) [plain]", q, "compared as multisets\nSQLite: 3 rows\nTurso: 4 rows\n", "", &[]), Some("9849"));
        // rows_differ__row_count___json_window_subquery_sum_cast_/0
        let q = "SELECT (c0 << (~(~c0))) FROM main.t4 a0 WHERE (c0 = (SELECT string_agg(c0, ',' ORDER BY CAST(b0.c0 AS TEXT), b0.c0 NULLS LAST) FROM (SELECT * FROM t4 LIMIT -1) AS b0))";
        assert_eq!(query("rows-differ (row count) [json,window,subquery,sum,cast]", q, "compared as multisets\nSQLite: 0 rows\nTurso: 1 rows\n  0\n", "", &[]), Some("9849"));
        // A two-digit exponent is not this.
        assert_eq!(query("rows-differ [plain]", "SELECT * FROM t0 WHERE c0 > '1.5e20'", "SQLite: 1 rows\nTurso: 1 rows\n", "", &[]), None);
    }

    #[test]
    fn expressions_sqlite_never_evaluates_are_9851() {
        // query_error__integer_overflow/0: a literal in ORDER BY, no rows.
        let q = "SELECT * FROM main.t1 AS a0 WHERE (c0 > 1) ORDER BY abs(-9223372036854775808) DESC, 5 NULLS LAST, a0.rowid DESC LIMIT 4 OFFSET 1";
        assert_eq!(query("query-error: integer overflow", q, "SQLite: 0 rows\nTurso: error: integer overflow\n", "integer overflow", &[]), Some("9851"));
        // query_error__malformed_JSON/1: a guarded BLOB literal, WHERE NULL.
        let q = "SELECT max(CAST((json_extract((CASE WHEN json_valid(x'c4143078464646464646464646464646464646464646', 8) THEN x'c4143078464646464646464646464646464646464646' END), '$') || json_type(json_object())) AS TEXT)) FROM x0 a0 WHERE NULL";
        assert_eq!(query("query-error: malformed JSON", q, "SQLite: 1 rows\n  NULL\nTurso: error: malformed JSON\n", "malformed JSON", &[]), Some("9851"));
        // query_error__malformed_JSON/2: a text literal before `->>`.
        let q = "SELECT min(CAST(a3.c1 AS INTEGER)) FILTER (WHERE ('{''\\u00e9'':0xFFFFFFFFFFFFFFFFFFFF,\"0\":\"\\''\",}' ->> \"$.é\")) OVER (PARTITION BY a3.c0) AS r2 FROM main.t1 AS a0 FULL OUTER JOIN main.t3 AS a3 ON (a3.c0 = a0.c0)";
        assert_eq!(query("query-error: malformed JSON", q, "SQLite: 0 rows\nTurso: error: malformed JSON\n", "malformed JSON", &[]), Some("9851"));
        // query_error__malformed_JSON/0: json_tree of an outer column in a
        // scalar subquery, over a join with an empty table (SQLite: 1 row).
        let q = "SELECT (SELECT count(DISTINCT value) FROM json_tree((CASE WHEN json_valid(a0.c4, 10) THEN a0.c4 END))), quote(zeroblob(length(-3))) FROM t0 AS a0 CROSS JOIN main.t3 AS a1 JOIN t3 AS a2 ON (a2.c0 = a1.c0)";
        assert_eq!(query("query-error: malformed JSON", q, "SQLite: 1 rows\n  1|NULL|\"X'0000'\"|0\nTurso: error: malformed JSON\n", "malformed JSON", &[]), Some("9851"));
        // Rows SQLite returned, no literal: not this.
        let q = "SELECT json_extract(c0, '$') FROM t0";
        assert_eq!(query("query-error: malformed JSON", q, "SQLite: 2 rows\n  1\n  2\nTurso: error: malformed JSON\n", "malformed JSON", &[]), None);
    }

    #[test]
    fn aggregate_two_subqueries_down_is_9519() {
        let q = "WITH x0 AS (SELECT cid AS c0, name AS c1 FROM pragma_table_info('t5')) SELECT replace((NOT EXISTS (SELECT (c1 < c0) FROM t3 AS b0 WHERE (c0 <> (SELECT count(c4) FROM main.t2 AS d0 WHERE (d0.c0 > 1))))), '', '') FROM x0 AS a0";
        let exact = format!("misuse of aggregate: count() in {q} at offset 140");
        let sig = "query-accepted: misuse of aggregate: count() in WITH xN AS (SELECT cid AS cN, name AS cN FROM pragma_table_info('?')) SELECT";
        assert_eq!(query(sig, q, "SQLite: error: misuse of aggregate: count()\nTurso: 1 rows\n", &exact, &[]), Some("9519"));
        // One level down: not this rule.
        let q = "SELECT (SELECT 1 FROM t3 WHERE count(c0) > 1) FROM t0";
        assert_eq!(query("query-accepted: misuse of aggregate: count()", q, "SQLite: error\nTurso: 1 rows\n", "misuse of aggregate: count()", &[]), None);
    }

    #[test]
    fn upsert_on_a_renamed_without_rowid_table_is_documented() {
        // statement_error__Parse_error__ON_CONFLICT_clause../0: t0 (WITHOUT
        // ROWID in the file) becomes t2 through the t_swap triple.
        let stmts = strings(&["SAVEPOINT s0", "ALTER TABLE t2 RENAME TO t_swap; ALTER TABLE t0 RENAME TO t2; ALTER TABLE t_swap RENAME TO t0", "ROLLBACK"]);
        let wr = without_rowid_in_statements(vec!["t0".into()], &stmts);
        let insert = "INSERT INTO t2 (c4, c0, c1, c2) VALUES (-253205912, 1711618200, 'Az0', x'6b') ON CONFLICT (c4) DO UPDATE SET c1 = excluded.c1, c2 = excluded.c2";
        assert!(documented_without_rowid("Parse error: ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint"));
        assert!(writes_without_rowid(&wr, insert));
        assert!(!writes_without_rowid(&without_rowid_in_statements(vec!["t0".into()], &[]), insert));
        // A table a statement creates WITHOUT ROWID, then renames.
        let wr = without_rowid_in_statements(Vec::new(), &strings(&["CREATE TABLE t4(c0 PRIMARY KEY) WITHOUT ROWID", "ALTER TABLE \"t4\" RENAME TO t9"]));
        assert_eq!(wr, vec!["t4".to_string(), "t9".to_string()]);
    }

    #[test]
    fn local_drafts_and_the_jsonb_candidate() {
        // turso-multi-index-or-stale-deferred-seek (UNIQUE_constraint_failed/0)
        let d = "statement 2: INSERT INTO t0 (c0) VALUES (3);\nSQLite: ok\nTurso: error: UNIQUE constraint failed: t0.c0\n";
        let stmts = ["PRAGMA secure_delete = FAST", "DELETE FROM t0 AS d WHERE ((c1 BETWEEN 9.223372036854776e18 AND 1.0822561597398664e30 AND c0 < 43) OR c0 BETWEEN -5.75 AND '١٢') RETURNING c1, c1"];
        assert_eq!(statement("statement-error: UNIQUE constraint failed: tN.cN", d, "UNIQUE constraint failed: t0.c0", &stmts), Some(DRAFT_DEFERRED_SEEK));
        assert_eq!(statement("statement-error: UNIQUE constraint failed: tN.cN", d, "UNIQUE constraint failed: t0.c0", &stmts[..1]), None);
        // turso-json-blob-arg-jsonb-detection (Unexpected_end_of_input/0)
        let q = "SELECT ((CASE WHEN json_valid(c0, 10) THEN c0 END) ->> NULL) FROM t2 AS a0";
        let rest = format!("SQLite: 1 rows\n  NULL\nTurso: error: Parse error: Unexpected end of input\n{NOTE_JSONB_WHITESPACE}");
        assert_eq!(query("query-error: Parse error: Unexpected end of input", q, &rest, "Parse error: Unexpected end of input", &[]), Some(DRAFT_JSON_BLOB));
        // turso-jsonb-text-non-utf8-rejected (Parse_error__malformed_JSON/0)
        let q = "SELECT count(*) FROM (t2 AS a0 INNER JOIN t1 AS a1 ON (a1.c4 >= a0.c1)) WHERE (((CASE WHEN json_valid(a1.c0, 11) THEN a1.c0 END) -> 3) <> '😀011')";
        let rest = format!("SQLite: 1 rows\n  19\nTurso: error: Parse error: malformed JSON\n{NOTE_JSONB_NOT_UTF8}");
        assert_eq!(query("query-error: Parse error: malformed JSON", q, &rest, "Parse error: malformed JSON", &[]), Some(CANDIDATE_JSONB_UTF8));
        // known/10352: the same class, which the group_concat-over rule used
        // to claim; that rule is for differing rows now.
        let q = "SELECT group_concat(c4) OVER (ORDER BY rowid ROWS 2 PRECEDING), ((CASE WHEN json_valid(c5, 10) THEN c5 END) ->> '$.a') FROM t2";
        let rest = format!("SQLite: 2 rows\n  NULL|NULL\nTurso: error: Parse error: malformed JSON\n{NOTE_JSONB_NOT_UTF8}");
        assert_eq!(query("query-error: Parse error: malformed JSON", q, &rest, "Parse error: malformed JSON", &[]), Some(CANDIDATE_JSONB_UTF8));
        let rest = "SQLite: 2 rows\n  NULL|NULL\nTurso: error: Parse error: malformed JSON\n";
        assert_eq!(query("query-error: Parse error: malformed JSON", q, rest, "Parse error: malformed JSON", &[]), None);
    }

    #[test]
    fn keyword_rules_do_not_claim_errors() {
        // "#10426: root path should be text" (46 in the run): #9838.
        let q = "SELECT replace(c0, '', NULL), (SELECT count(*) FROM json_tree('[1]', NULL)) FROM t0";
        assert_eq!(query("query-error: root path should be text", q, "SQLite: 1 rows\nTurso: error: root path should be text\n", "root path should be text", &[]), Some("9838"));
        // likelihood() in a query with soundex(): #10338.
        let q = "SELECT soundex(c0), likelihood(c1, 1e-1) FROM t0";
        let sig = "query-error: Parse error: second argument to likelihood() must be a floating point number with decimal";
        assert_eq!(query(sig, q, "SQLite: 1 rows\nTurso: error\n", "", &[]), Some("10338"));
        // known/5769: insufficient registers with BETWEEN over pragma_index_xinfo: #9051.
        let q = "WITH x0 AS MATERIALIZED (SELECT v1.name AS c3 FROM pragma_index_list('t1') AS v0, pragma_index_xinfo(v0.name) AS v1) SELECT * FROM t0 AS a0 LEFT JOIN x0 AS a1 ON ((a0.\"c3\", a1.\"c3\") BETWEEN (1, 2) AND (3, 4))";
        let sig = "query-error: Parse error: insufficient registers allocated for expression vector write (start=N, count=";
        assert_eq!(query(sig, q, "SQLite: 0 rows\nTurso: error: (\n", "", &[]), Some("9051"));
        // known/9837: a trailing date modifier in a query with a JSON parse error.
        let q = "SELECT date(c0, '+1 day x'), json_extract(c1, '$') FROM t0";
        assert_eq!(query("query-error: Parse error: malformed JSON", q, "SQLite: 2 rows\n  NULL|1\nTurso: error: Parse error: malformed JSON\n", "", &[]), None);
        // known/9885 style: json_pretty in a query SQLite rejected.
        let q = "SELECT json_pretty('{}') FROM x0 a0 RIGHT JOIN t3 a1 ON NULL JOIN x0 a2 ON a2.c2";
        assert_eq!(query("query-accepted: ON clause references tables to its right", q, "SQLite: error: ON clause references tables to its right\nTurso: 0 rows\n", "", &[]), None);
        // The keyword rules still apply to differing rows.
        assert_eq!(query("rows-differ [plain]", "SELECT replace(c0, '', NULL) FROM t0", "SQLite: 1 rows\n  'a'\nTurso: 1 rows\n  NULL\n", "", &[]), Some("10426"));
    }

    #[test]
    fn jsonb_classification() {
        // TEXT and TEXT5 elements whose payload is not UTF-8.
        assert!(jsonb_text_not_utf8(&[0x47, 0xE2, 0x21, 0x50, 0x2E]));
        assert!(jsonb_text_not_utf8(&[0x29, 0x99, 0x9A]));
        // An array of 'ab', and of a non-UTF-8 text.
        assert!(!jsonb_text_not_utf8(&[0x3B, 0x27, b'a', b'b']));
        assert!(jsonb_text_not_utf8(&[0x3B, 0x27, 0xE2, 0x21]));
        assert!(!jsonb_text_not_utf8(&[0x0A]));
        let db = sqlite_memory().unwrap();
        db.execute_batch("CREATE TABLE t(c); INSERT INTO t VALUES (x'47E221502E'), (x'0A'), ('x'), (x'3B276162');").unwrap();
        let n = jsonb_notes(&db);
        assert!(n.contains(NOTE_JSONB_NOT_UTF8) && n.contains(NOTE_JSONB_WHITESPACE), "{n}");
        db.execute_batch("DELETE FROM t WHERE c <> x'3B276162'").unwrap();
        assert_eq!(jsonb_notes(&db), "");
    }

    #[test]
    fn helpers() {
        assert!(jsonb_guard("case when json_valid(a1.c0, 11) then a1.c0 end"));
        assert!(!jsonb_guard("case when json_valid(a1.c0) then a1.c0 end"));
        assert!(!jsonb_guard("json_valid(json_extract(c0, '$'), 2)"));
        let script = "PRAGMA a = 1;\nINSERT INTO t VALUES ('x\ny');\nALTER TABLE t DROP c; ALTER TABLE t RENAME d TO c;\n";
        assert_eq!(script_entries(script), vec!["PRAGMA a = 1", "INSERT INTO t VALUES ('x\ny')", "ALTER TABLE t DROP c; ALTER TABLE t RENAME d TO c"]);
        assert_eq!(dropped_column_tables(&strings(&["ALTER TABLE [T3] DROP COLUMN C2; ALTER TABLE t3 RENAME COLUMN c3 TO `c2`", "ALTER TABLE t4 DROP CONSTRAINT k"])), vec!["t3"]);
        assert!(min_int_default(&strings(&["ALTER TABLE t ADD c INT DEFAULT (-9223372036854775808)"])));
        assert_eq!(with_clause("with x0 as (select 1), x1(c0) as not materialized (select (2)) select * from x1").1, vec!["x0", "x1"]);
    }
}
