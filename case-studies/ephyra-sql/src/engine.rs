// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Running SQL in Turso (through `turso_core`, synchronously) and in
//! SQLite, and comparing what they return.

use std::sync::Arc;

use turso_core::{Database, MemoryIO, Numeric, SqliteDialect};

use crate::ast::Case;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    /// Never NaN (both engines turn it into NULL); `-0.0` is stored as `0.0`.
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

pub type Row = Vec<Value>;

/// What a query returned. Error messages are kept for the report but not
/// compared: Turso's error kinds are too coarse to match SQLite's.
#[derive(Clone, Debug)]
pub enum Outcome {
    Rows(Vec<Row>),
    Error(String),
}

fn real(x: f64) -> Value {
    Value::Real(if x == 0.0 { 0.0 } else { x })
}

/// A Turso in-memory database with one connection, for running statements
/// one after another. A Turso panic propagates, so the property fails with
/// it.
pub struct Turso {
    _db: Arc<Database>,
    conn: Arc<turso_core::Connection>,
}

impl Turso {
    pub fn open() -> Self {
        let io = Arc::new(MemoryIO::new());
        let db = Database::open_file(io, ":memory:", Arc::new(SqliteDialect))
            .expect("an in-memory database opens");
        let conn = db.connect().expect("an in-memory database connects");
        Turso { _db: db, conn }
    }

    pub fn run(&self, sql: &str) -> Outcome {
        let run = || -> turso_core::Result<Vec<Row>> {
            let mut stmt = self.conn.prepare(sql)?;
            let mut rows = Vec::new();
            stmt.run_with_row_callback(|row| {
                rows.push(row.get_values().map(from_turso).collect());
                Ok(())
            })?;
            Ok(rows)
        };
        run().map_or_else(|e| Outcome::Error(e.to_string()), Outcome::Rows)
    }

    /// Rows changed by the last write.
    pub fn changes(&self) -> i64 {
        self.conn.changes()
    }
}

fn from_turso(v: &turso_core::Value) -> Value {
    match v {
        turso_core::Value::Null => Value::Null,
        turso_core::Value::Numeric(Numeric::Integer(n)) => Value::Integer(*n),
        turso_core::Value::Numeric(Numeric::Float(x)) => real(f64::from(*x)),
        turso_core::Value::Text(t) => Value::Text(t.as_str().to_string()),
        turso_core::Value::Blob(b) => Value::Blob(b.as_slice().to_vec()),
    }
}

/// An SQLite in-memory database, the reference.
pub struct Sqlite(rusqlite::Connection);

impl Sqlite {
    pub fn open() -> Self {
        Sqlite(rusqlite::Connection::open_in_memory().expect("an in-memory database opens"))
    }

    pub fn run(&self, sql: &str) -> Outcome {
        let run = || -> rusqlite::Result<Vec<Row>> {
            let mut stmt = self.0.prepare(sql)?;
            let n = stmt.column_count();
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(
                    (0..n).map(|i| row.get_ref(i).map(from_sqlite)).collect::<Result<_, _>>()?,
                );
            }
            Ok(out)
        };
        run().map_or_else(|e| Outcome::Error(e.to_string()), Outcome::Rows)
    }

    /// Rows changed by the last write.
    pub fn changes(&self) -> i64 {
        self.0.changes() as i64
    }
}

/// A case in a fresh Turso database: its setup, then its query.
pub fn turso(case: &Case) -> Outcome {
    let db = Turso::open();
    run_case(case, |sql| db.run(sql))
}

/// A case in a fresh SQLite database.
pub fn sqlite(case: &Case) -> Outcome {
    let db = Sqlite::open();
    run_case(case, |sql| db.run(sql))
}

/// A failing setup statement is the case's outcome.
fn run_case(case: &Case, run: impl Fn(&str) -> Outcome) -> Outcome {
    for statement in case.setup() {
        if let Outcome::Error(e) = run(&statement) {
            return Outcome::Error(format!("setup: {e}"));
        }
    }
    run(&case.select())
}

fn from_sqlite(v: rusqlite::types::ValueRef) -> Value {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => Value::Integer(n),
        ValueRef::Real(x) => real(x),
        ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Blob(b.to_vec()),
    }
}

/// Both errored, or both returned the same rows: in order when `ordered`,
/// else as multisets. REALs compare exactly.
pub fn agree(ordered: bool, a: &Outcome, b: &Outcome) -> bool {
    match (a, b) {
        (Outcome::Error(_), Outcome::Error(_)) => true,
        (Outcome::Rows(x), Outcome::Rows(y)) if ordered => x == y,
        (Outcome::Rows(x), Outcome::Rows(y)) => sorted(x) == sorted(y),
        _ => false,
    }
}

/// Rows in one canonical order, so multisets compare as vectors. The key is
/// exact: `Debug` prints REALs round-trip and tells INTEGER 1 from REAL 1.0.
fn sorted(rows: &[Row]) -> Vec<Row> {
    let mut rows = rows.to_vec();
    rows.sort_by_cached_key(|row| format!("{row:?}"));
    rows
}
