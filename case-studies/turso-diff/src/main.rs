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
//! What is not a difference is counted, by reason: a case SQLite rejects
//! (the generator follows SQLite trunk, so a few statements are newer than
//! 3.50.4), and a statement or query Turso rejects as unsupported. A Turso
//! panic, a query only one engine fails, rows that differ, and a statement
//! only Turso fails for another reason are findings, kept per signature
//! (at most [`EXAMPLES`] each) and shrunk with the generator's shrinker.
//! A worker stuck on one input for [`HANG`] is a hang finding.
//!
//! `cargo run --release -- --seed S --minutes M --jobs J --out DIR [--experimental]`

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
    cases::Case,
    SqliteFile,
    expr::{Query, Schema},
    probes::{case_queries, final_schema},
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
}

fn past_deadline() -> bool {
    DEADLINE.with(|d| d.get().is_some_and(|t| Instant::now() > t))
}

fn sqlite_budget() {
    DEADLINE.with(|d| d.set(Some(Instant::now() + SQLITE_BUDGET)));
}

/// The case's file in memory, its statements run; or the first error.
fn open_sqlite(case: &Case) -> Result<Connection, String> {
    let mut db = Connection::open_in_memory().map_err(|e| e.to_string())?;
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
    let mut db = Connection::open_in_memory().map_err(|e| e.to_string())?;
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
    let dst = Connection::open_in_memory().map_err(e)?;
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
/// names a dropped index then fails on SQLite, and the input is skipped.
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
    let mut db = Connection::open_in_memory().map_err(|e| e.to_string())?;
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
/// as rusqlite's bundled build is and Turso's default-build target is not.
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
        _ => false,
    })
}

fn negative_zero_cells(rows: &Rows) -> bool {
    rows.iter().flatten().any(|c| matches!(c, Cell::Real(x) if *x == 0.0 && x.is_sign_negative()))
}

/// The trunk-oracle helper (SQLite trunk, a separate program: two SQLite
/// versions can't be linked into one).
const TRUNK_ORACLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/trunk-oracle/target/release/trunk-oracle");

fn unhex(h: &str) -> Vec<u8> {
    (0..h.len() / 2).filter_map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()).collect()
}

/// SQLite trunk's result for the finding's query, after the case's
/// statements and the queries before it; `None` if the helper is missing
/// or fails.
fn trunk_result(input: &Input, f: &Finding, dir: &Path) -> Option<Result<Rows, String>> {
    let k = f.query?;
    if !Path::new(TRUNK_ORACLE).exists() {
        return None;
    }
    let case = prepared(&input.0).ok()?;
    let n = SERIAL.fetch_add(1, Ordering::Relaxed);
    let (db, sql) = (dir.join(format!("trunk-{n}.db")), dir.join(format!("trunk-{n}.sql")));
    fs::write(&db, &case.file.0).ok()?;
    fs::write(&sql, format!("{}{}", script(&case), queries_sql(&input.1, Some(k)))).ok()?;
    let out = std::process::Command::new(TRUNK_ORACLE).arg(&db).arg(&sql).output();
    let _ = fs::remove_file(&db);
    let _ = fs::remove_file(&sql);
    let text = String::from_utf8_lossy(&out.ok()?.stdout).into_owned();
    let mut lines = text.lines();
    let head = lines.next()?;
    if let Some(e) = head.strip_prefix("E:") {
        return Some(Err(e.to_string()));
    }
    if head != "OK" {
        return None;
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
    Some(Ok(rows))
}

/// Whether SQLite trunk gives what Turso gave: then the difference is a
/// SQLite 3.50.4 behavior that newer SQLite changed to Turso's, not a Turso
/// bug (an error in both counts as agreeing).
fn trunk_agrees(input: &Input, f: &Finding, dir: &Path) -> bool {
    let Some(Some(turso)) = f.query.map(|_| f.turso.clone()) else { return false };
    let Some(trunk) = trunk_result(input, f, dir) else { return false };
    match (turso, trunk) {
        (Err(_), Err(_)) => true,
        (Ok(a), Ok(b)) => {
            let (a, b) = if f.ordered { (a, b) } else { (sorted(&a), sorted(&b)) };
            a == b || json_digits(&a) == json_digits(&b)
        },
        _ => false,
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

/// A simple CASE (`CASE x WHEN ...`), whose comparisons Turso makes without
/// SQLite's affinity rules (turso #8225).
fn simple_case(l: &str) -> bool {
    l.match_indices("case ").any(|(i, m)| !l[i + m.len()..].trim_start().starts_with("when"))
}

/// Filed issues fixed on the Turso commit under test (checked by rerunning
/// each repro): a finding a rule would attribute to one of these is a
/// regression or another bug, so it is reported, not labelled known.
const FIXED: &[&str] = &["9839", "9884", "9887", "9889", "9921", "9924", "9925", "9926", "9927", "9932", "9934", "9935"];

fn known(f: &Finding, sql: &str) -> Option<&'static str> {
    known_rules(f, sql).into_iter().find(|i| !FIXED.contains(i))
}

/// The filed Turso issue a finding matches, by its signature and its query:
/// matched findings keep one example each under `findings/known/`, unshrunk.
/// The rules are deliberately narrow; an example per issue still shows
/// what they catch.
fn known_rules(f: &Finding, sql: &str) -> Vec<&'static str> {
    let mut hits: Vec<&'static str> = Vec::new();
    let sig = f.signature.as_str();
    let l = sql.to_lowercase();
    let any = |ps: &[&str]| ps.iter().any(|p| l.contains(p));
    let joins = any(&["right join", "full join"]);
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
        if sig.contains("GroupConcat accumulator is Text") {
            hits.push("10352");
        }
        if any(&[" over ", " over("]) && (sig.contains("must be a buffer") || sig.contains("first_value") || sig.contains("lag/lead")) {
            hits.push("8336");
        }
        return hits;
    }
    if sig.starts_with("statement-error") {
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
        if sig.contains("cannot drop column") && sig.contains("referenced in VIEW") {
            hits.push("4009");
        }
        if sig.contains("after rename column") || (sig.contains("error in trigger") && any(&["rename", "drop column"])) {
            hits.push("7644");
        }
        return hits;
    }
    // Before the broad rules below: [`known`] takes the first match.
    if sig.starts_with("rows-differ") || sig.starts_with("query-error") {
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
        if l.contains("soundex(") {
            hits.push("10356");
        }
        if sig.contains("second argument to likelihood()") {
            hits.push("10338");
        }
        if sig.contains("insufficient registers allocated for expression vector") && l.contains("between") {
            hits.push("9051");
        }
    }
    if sig.contains("root path should be text") {
        hits.push("9838");
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
    if ((sig.starts_with("query-error") && !sig.contains("Parse error")) || sig.starts_with("rows-differ")) && l.contains("group by") && l.contains("having") {
        hits.push(if sig.starts_with("query-error") { "9880" } else { "9887" });
    }
    if f.detail.contains("#6749 shape") && (sig.contains("UNIQUE constraint failed") || (sig.starts_with("rows-differ") && f.detail.contains("reads a WITHOUT ROWID table"))) {
        hits.push("6749");
    }
    if sig.starts_with("rows-differ") && l.contains(" join ") && f.detail.contains("reads a WITHOUT ROWID table") && (sig.contains("row count") || l.contains("count(")) {
        hits.push("9081");
    }
    if sig.starts_with("query-error: integer overflow") && l.contains("(select") {
        hits.push("9851");
    }
    if sig.contains("no such table: xN") && l.contains(" on ") {
        hits.push("9073");
    }
    if sig.starts_with("query-error: Parse error: no such column") && l.trim_start().starts_with("with") {
        hits.push("9842");
    }
    if sig.contains("UNIQUE constraint failed") && f.detail.to_lowercase().contains("without rowid") {
        hits.push("6749");
    }
    if sig.starts_with("rows-differ") || sig.starts_with("query-error") || sig.starts_with("query-accepted") {
        if trailing_date_modifier(sql) {
            hits.push("9837");
        }
        if any(&["json_insert(", "jsonb_insert("]) && l.contains("'$'") {
            hits.push("9924");
        }
        if any(&["json_set((case when json_valid", "jsonb_set((case when json_valid", "json_insert((case when json_valid", "jsonb_insert((case when json_valid", "json_replace((case when json_valid", "jsonb_replace((case when json_valid"]) {
            hits.push("9844");
        }
        if any(&["'24:", "'+24:", "'-24:"]) {
            hits.push("9921");
        }
        if simple_case(&l) && sig.starts_with("rows-differ") {
            hits.push("8225");
        }
        if l.contains("json_pretty") {
            hits.push("9885");
        }
        if l.contains("timediff") {
            hits.push("9886");
        }
        if l.contains("strftime") {
            hits.push(if l.contains("%j") { "9888" } else { "9884" });
        }
        if negative_zero(&f.detail) && any(&["json", "->"]) {
            hits.push("9840");
        }
        if any(&["json_patch", "jsonb_patch"]) {
            hits.push("9843");
        }
        if any(&["'$[#]'", "'$[#"]) && any(&["json_set", "jsonb_set", "json_insert", "jsonb_insert"]) {
            hits.push("8538");
        }
        if any(&["json_set(null", "jsonb_set(null", "json_insert(null", "jsonb_insert(null", "json_replace(null", "jsonb_replace(null", "json_remove(null", "jsonb_remove(null", "', null", "$', null"]) {
            hits.push("9844");
        }
        if any(&["json_set(", "jsonb_set(", "json_insert(", "jsonb_insert(", "json_replace(", "jsonb_replace(", "json_remove(", "jsonb_remove("]) && (sig.contains("JSON cannot hold BLOB") || f.detail.contains("NULL")) {
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
        if any(&["json_each", "json_tree"]) && l.trim_start().starts_with("with") && sig.starts_with("rows-differ") {
            hits.push("10342");
        }
        if sig.contains("malformed JSON") && (f.detail.contains("//") || l.contains("//")) {
            hits.push("10343");
        }
        if l.contains("group_concat") && l.contains(" over ") && f.detail.contains("NULL") {
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
    // order. Its write model panics on some statement lists left after a
    // failing statement was dropped (see the worker loop): skip those.
    let schema: Schema = match panic::catch_unwind(AssertUnwindSafe(|| final_schema(case))) {
        Ok(s) => s,
        Err(_) => {
            out.skips.push("generator model rejects the statements left after a drop".into());
            return out;
        },
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
    let notes = affinity_order_note(&sdb);
    for (k, s) in case.statements.iter().enumerate() {
        let sql = s.to_string();
        if let Err(e) = turso_exec(&t, &sql) {
            if unsupported(&e) {
                out.skips.push(format!("turso unsupported (statement): {}", normalize(&e)));
                out.failed_statement = Some(k);
            } else if documented_without_rowid(&e) && writes_without_rowid(&sdb, &sql) {
                out.skips.push(format!("turso documented (WITHOUT ROWID): {}", normalize(&e)));
                out.failed_statement = Some(k);
            } else {
                out.failed_statement = Some(k);
                let wr = if writes_without_rowid(&sdb, &sql) { "note: the statement names a WITHOUT ROWID table\n" } else { "" };
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
    let wr_tables = without_rowid_tables(&sdb);
    for (qi, q) in queries.iter().enumerate() {
        let sql = q.sql();
        out.queries += 1;
        // Function sets differ by design (Turso's extensions; SQLite's
        // build options), and so do the rows describing them.
        // page_count depends on how each engine lays out the file.
        if ["pragma_function_list", "pragma_page_count"].iter().any(|p| sql.to_lowercase().contains(p)) {
            out.skips.push("query reads pragma_function_list or pragma_page_count".into());
            continue;
        }
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
                out.findings.push(Finding {
                    signature,
                    detail: format!("{sql};\nSQLite: {}\nTurso: {}\n{note}", show(&s), show(&r)),
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

/// COMPAT.md: WITHOUT ROWID tables take plain INSERTs only (UPSERT is
/// rejected) and can't be a foreign key's parent.
fn documented_without_rowid(e: &str) -> bool {
    e.contains("ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint") || e.contains("foreign key mismatch")
}

/// `sql` names a WITHOUT ROWID table of `db` (its state after all the
/// statements, which is close enough for a note).
fn writes_without_rowid(db: &Connection, sql: &str) -> bool {
    without_rowid_tables(db).iter().any(|t| mentions(sql, &t.to_lowercase()))
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
        if f.query.is_some() && f.turso.is_some() && trunk_agrees(input, &f, dir) {
            o.skips.push("sqlite 3.50.4 quirk: sqlite trunk agrees with turso".into());
        } else {
            o.findings.push(f);
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

/// The smallest input the generator's shrinker finds that still gives a
/// finding of `signature`, within a minute.
fn shrink(input: &Input, target: &Finding, path: &Path, experimental: bool) -> (Input, Finding) {
    // Same signature and message, and the same known issue (or none): a
    // smaller input that lands on another, already filed bug is not this one.
    let sql_of = |i: &Input, f: &Finding| f.query.and_then(|k| i.1.get(k)).map(|q| q.sql()).unwrap_or_else(|| f.detail.clone());
    let target_known = known(target, &sql_of(input, target));
    let same_in = |c: &Input, f: &Finding| f.signature == target.signature && f.exact == target.exact && known(f, &sql_of(c, f)) == target_known;
    let signature = target.signature.as_str();
    let gen = case_queries();
    let mut best = input.clone();
    let mut why = None;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        let mut next = None;
        let _ = gen.shrink(&best, &mut |c| {
            if start.elapsed() >= Duration::from_secs(60) {
                return ControlFlow::Break(());
            }
            if let Some(f) = check(&c, path, experimental).findings.into_iter().find(|f| same_in(&c, f)) {
                next = Some((c, f));
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        });
        match next {
            Some((c, f)) => {
                best = c;
                why = Some(f);
            },
            None => break,
        }
    }
    let why = why.unwrap_or_else(|| check(&best, path, experimental).findings.into_iter().find(|f| same_in(&best, f)).unwrap_or(Finding { signature: signature.into(), detail: "(did not reproduce)".into(), ..Finding::default() }));
    (best, why)
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
}

fn slug(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    t.chars().take(80).collect()
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
    let sql = f.query.and_then(|k| input.1.get(k)).map(|q| q.sql()).unwrap_or_else(|| f.detail.clone());
    if let Some(issue) = known(f, &sql) {
        let ex = out.join("findings").join("known").join(issue);
        if ex.exists() {
            return;
        }
        let _ = fs::create_dir_all(&ex);
        let _ = fs::write(ex.join("signature.txt"), format!("{}\nmatched turso #{issue}\n", f.signature));
        let _ = fs::write(ex.join("case.db"), prepared(&input.0).map(|c| c.file.0).unwrap_or_else(|_| input.0.file.0.clone()));
        let _ = fs::write(ex.join("replay.sql"), format!("{}{}", script(&input.0), queries_sql(&input.1, f.query)));
        let _ = fs::write(ex.join("finding.txt"), format!("{}\n", f.detail));
        return;
    }
    let dir = out.join("findings").join(slug(&f.signature));
    let n = fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
    if n >= EXAMPLES {
        return;
    }
    let ex = dir.join(format!("{n}"));
    let _ = fs::create_dir_all(&ex);
    let _ = fs::write(ex.join("signature.txt"), format!("{}\n", f.signature));
    let _ = fs::write(ex.join("case.db"), prepared(&input.0).map(|c| c.file.0).unwrap_or_else(|_| input.0.file.0.clone()));
    let _ = fs::write(ex.join("statements.sql"), script(&input.0));
    let note = without_rowid_index_note(&input.0);
    let _ = fs::write(ex.join("finding.txt"), format!("{}\n{note}", f.detail));
    let _ = fs::write(ex.join("queries.sql"), queries_sql(&input.1, None));
    let _ = fs::write(ex.join("replay.sql"), format!("{}{}", script(&input.0), queries_sql(&input.1, f.query)));
    let (small, why) = shrink(input, f, path, experimental);
    let _ = fs::write(ex.join("shrunk-case.db"), prepared(&small.0).map(|c| c.file.0).unwrap_or_else(|_| small.0.file.0.clone()));
    let _ = fs::write(ex.join("shrunk-statements.sql"), script(&small.0));
    let _ = fs::write(ex.join("shrunk-queries.sql"), queries_sql(&small.1, None));
    let _ = fs::write(ex.join("shrunk-replay.sql"), format!("{}{}", script(&small.0), queries_sql(&small.1, why.query)));
    let note = without_rowid_index_note(&small.0);
    let _ = fs::write(ex.join("shrunk-finding.txt"), format!("{}\n{note}", why.detail));
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
    let bytes = fs::read(db).expect("database file");
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
    let mut sdb = Connection::open_in_memory().unwrap();
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--replay") {
        replay(Path::new(&args[i + 1]), Path::new(&args[i + 2]), args.iter().any(|a| a == "--experimental"));
        return;
    }
    let seed: u64 = arg(&args, "--seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let minutes: f64 = arg(&args, "--minutes").and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let jobs: u64 = arg(&args, "--jobs").and_then(|s| s.parse().ok()).unwrap_or(4);
    let Some(out) = arg(&args, "--out").map(PathBuf::from) else {
        eprintln!("usage: turso-diff --out DIR [--seed S] [--minutes M] [--jobs J] [--experimental]\n       turso-diff --replay DB SQLFILE [--experimental]");
        std::process::exit(2);
    };
    let experimental = args.iter().any(|a| a == "--experimental");
    fs::create_dir_all(out.join("tmp")).expect("output directory");
    // Turso panics are findings; keep their default messages off stderr.
    panic::set_hook(Box::new(|_| {}));

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
                            for f in &o.findings {
                                let sql = f.query.and_then(|k| input.1.get(k)).map(|q| q.sql()).unwrap_or_else(|| f.detail.clone());
                                let key = match known(f, &sql) {
                                    Some(issue) => format!("known turso #{issue}: {}", f.signature),
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
                                // would fail next, one retry each.
                                if let Some(name) = created_name(&gone) {
                                    let mut rest = input.0.statements.split_off(k);
                                    rest.retain(|s| !mentions(&s.to_string(), &name));
                                    input.0.statements.extend(rest);
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
