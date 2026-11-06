// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A minimal writable native virtual table for the round-3 repros (copied from ../repro2/src/lib.rs):
//! `CREATE VIRTUAL TABLE v USING mem` with columns (a, b); rows in memory;
//! begin() copies the rows, commit() drops the copy, rollback() restores
//! it (the pattern of `RowsTable` in core/vdbe/native_extension_tests.rs).
//! Every hook call is printed.

use std::sync::{Arc, Mutex};
use turso_core::{
    native_ext::{ConstraintInfo, ConstraintUsage, IndexInfo, OrderByInfo, TableUpdate, VTabKind, VirtualTable, VirtualTableCursor, VirtualTableModule},
    Connection, ConnectOpts, Database, IOResult, LimboError, OpenOptions, PlatformIO, StepResult, Value,
};

type Rows = Vec<(i64, Value, Value)>;
type IOResultOr<T> = std::result::Result<IOResult<T>, Box<LimboError>>;

#[derive(Debug, Default)]
pub struct Store {
    rows: Mutex<Rows>,
    before: Mutex<Option<Rows>>,
}

#[derive(Debug)]
pub struct Mem(pub Arc<Store>);
#[derive(Debug)]
pub struct MemTable(Arc<Store>);

impl VirtualTableModule for Mem {
    type Table = MemTable;
    fn schema(&self, _a: &[Value]) -> turso_core::Result<String> {
        Ok("CREATE TABLE x(a, b)".into())
    }
    fn create(&self, _a: &[Value]) -> turso_core::Result<MemTable> {
        Ok(MemTable(self.0.clone()))
    }
}

impl VirtualTable for MemTable {
    type Cursor = MemCursor;
    fn open(&self, _c: Arc<Connection>) -> turso_core::Result<MemCursor> {
        Ok(MemCursor { store: self.0.clone(), rows: Vec::new(), pos: 0 })
    }
    fn best_index(&self, c: &[ConstraintInfo], _o: &[OrderByInfo]) -> Result<IndexInfo, turso_ext::ResultCode> {
        Ok(IndexInfo { constraint_usages: c.iter().map(|_| ConstraintUsage { argv_index: None, omit: false }).collect(), ..Default::default() })
    }
    fn readonly(&self) -> bool {
        false
    }
    fn create_update(&mut self, args: Vec<Value>) -> turso_core::Result<Box<dyn TableUpdate>> {
        Ok(Box::new(MemUpdate(self.0.clone(), args)))
    }
    fn begin(&mut self) -> turso_core::Result<()> {
        println!("  [module] begin");
        *self.0.before.lock().unwrap() = Some(self.0.rows.lock().unwrap().clone());
        Ok(())
    }
    fn commit(&mut self) -> turso_core::Result<()> {
        println!("  [module] commit");
        *self.0.before.lock().unwrap() = None;
        Ok(())
    }
    fn rollback(&mut self) -> turso_core::Result<()> {
        println!("  [module] rollback");
        if let Some(b) = self.0.before.lock().unwrap().take() {
            *self.0.rows.lock().unwrap() = b;
        }
        Ok(())
    }
}

pub struct MemCursor {
    store: Arc<Store>,
    rows: Rows,
    pos: usize,
}

impl VirtualTableCursor for MemCursor {
    fn filter(&mut self, _a: &[Value], _s: Option<&str>, _n: i32) -> IOResultOr<bool> {
        self.rows = self.store.rows.lock().unwrap().clone();
        self.pos = 0;
        Ok(IOResult::Done(!self.rows.is_empty()))
    }
    fn next(&mut self) -> IOResultOr<bool> {
        self.pos += 1;
        Ok(IOResult::Done(self.pos < self.rows.len()))
    }
    fn column(&mut self, c: usize) -> IOResultOr<Value> {
        let r = &self.rows[self.pos];
        Ok(IOResult::Done(if c == 0 { r.1.clone() } else { r.2.clone() }))
    }
    fn rowid(&self) -> i64 {
        self.rows[self.pos].0
    }
}

struct MemUpdate(Arc<Store>, Vec<Value>);

fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Numeric(turso_core::Numeric::Integer(i)) => Some(*i),
        _ => None,
    }
}

impl TableUpdate for MemUpdate {
    /// [rowid, NULL] = delete; [NULL, rowid?, a, b] = insert;
    /// [old, new, a, b] = update.
    fn step(&mut self) -> IOResultOr<Option<i64>> {
        let a = &self.1;
        let mut rows = self.0.rows.lock().unwrap();
        if a.len() == 2 {
            let old = int(&a[0]);
            rows.retain(|r| Some(r.0) != old);
            println!("  [module] delete {old:?}");
            return Ok(IOResult::Done(None));
        }
        let old = int(&a[0]);
        let new = int(&a[1]).or(old).unwrap_or_else(|| rows.iter().map(|r| r.0).max().unwrap_or(0) + 1);
        rows.retain(|r| Some(r.0) != old);
        rows.push((new, a[2].clone(), a[3].clone()));
        rows.sort_by_key(|r| r.0);
        println!("  [module] {} rowid {new}", if old.is_some() { "update" } else { "insert" });
        Ok(IOResult::Done(if old.is_none() { Some(new) } else { None }))
    }
}

/// The database (for more connections) at `path`, module registered as `mem`.
pub fn open_db(path: &str) -> Arc<Database> {
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{path}{ext}"));
    }
    let opts = OpenOptions::new(Arc::new(turso_sqlite::SqliteDialect)).native_module("mem", VTabKind::VirtualTable, Mem(Arc::default()));
    let io = Arc::new(PlatformIO::new().unwrap());
    Database::open(io, path, opts).unwrap()
}

pub fn connect(db: &Arc<Database>) -> Arc<Connection> {
    db.connect(ConnectOpts::new()).unwrap()
}

/// A database at `path` opened with ATTACH enabled (DatabaseOpts::with_attach).
pub fn open_db_attach(path: &str) -> Arc<Database> {
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{path}{ext}"));
    }
    let opts = OpenOptions::new(Arc::new(turso_sqlite::SqliteDialect)).db_opts(turso_core::DatabaseOpts::new().with_attach(true));
    let io = Arc::new(PlatformIO::new().unwrap());
    Database::open(io, path, opts).unwrap()
}

/// Like [`exec`], prefixed with the connection's name.
pub fn on(name: &str, c: &Arc<Connection>, sql: &str) {
    print!("{name}: ");
    exec(c, sql);
}

/// A database at `path` with the module registered as `mem`.
pub fn open(path: &str) -> Arc<Connection> {
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{path}{ext}"));
    }
    let opts = OpenOptions::new(Arc::new(turso_sqlite::SqliteDialect)).native_module("mem", VTabKind::VirtualTable, Mem(Arc::default()));
    let io = Arc::new(PlatformIO::new().unwrap());
    let db = Database::open(io, path, opts).unwrap();
    db.connect(ConnectOpts::new()).unwrap()
}

/// Runs one statement to the end and prints its rows or error.
pub fn exec(c: &Arc<Connection>, sql: &str) {
    println!("{sql}");
    let mut st = match c.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            println!("  -> prepare error: {e}");
            return;
        },
    };
    loop {
        match st.step() {
            Ok(StepResult::Row) => {
                let r = st.row().unwrap();
                let v: Vec<String> = r.get_values().map(|v| format!("{v}")).collect();
                println!("  -> row {}", v.join("|"));
            },
            Ok(StepResult::Done) => break,
            Ok(StepResult::IO) => st._io().step().unwrap(),
            Ok(other) => {
                println!("  -> {other:?}");
                break;
            },
            Err(e) => {
                println!("  -> error: {e}");
                break;
            },
        }
    }
}

pub fn state(c: &Arc<Connection>) {
    println!("  changes()={} total_changes()={} last_insert_rowid()={}", c.changes(), c.total_changes(), c.last_insert_rowid());
}

/// Round 3: a database at `path` in MVCC mode (DatabaseOpts::with_mvcc).
pub fn open_db_mvcc(path: &str) -> Arc<Database> {
    for ext in ["", "-wal", "-shm", "-log"] {
        let _ = std::fs::remove_file(format!("{path}{ext}"));
    }
    let opts = OpenOptions::new(Arc::new(turso_sqlite::SqliteDialect)).db_opts(turso_core::DatabaseOpts::new().with_mvcc(true));
    let io = Arc::new(PlatformIO::new().unwrap());
    Database::open(io, path, opts).unwrap()
}

/// Steps a prepared statement once and prints the result.
pub fn step_once(name: &str, st: &mut turso_core::Statement) {
    loop {
        match st.step() {
            Ok(StepResult::Row) => {
                let v: Vec<String> = st.row().unwrap().get_values().map(|v| format!("{v}")).collect();
                println!("{name}: step -> row {}", v.join("|"));
                return;
            },
            Ok(StepResult::Done) => {
                println!("{name}: step -> done");
                return;
            },
            Ok(StepResult::IO) => st._io().step().unwrap(),
            Ok(other) => {
                println!("{name}: step -> {other:?}");
                return;
            },
            Err(e) => {
                println!("{name}: step -> error: {e}");
                return;
            },
        }
    }
}
