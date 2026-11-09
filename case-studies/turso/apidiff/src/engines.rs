// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The two engines behind one interface: SQLite 3.50.4 through raw
//! libsqlite3-sys calls, Turso through `turso_core` (Connection::prepare,
//! Statement::step/reset/bind_at, StepResult). Each holds up to two
//! connections on one database file and a table of statement slots.
//!
//! Both register the same application functions ([`APP_FUNCTIONS`]):
//! SQLite through `sqlite3_create_function_v2` / `sqlite3_create_window_function`
//! reading arguments with `sqlite3_value_*`, Turso through its native
//! extension API (`ExternalFunc::new_native_scalar` / `new_native_aggregate`).

use std::{
    collections::BTreeMap,
    ffi::{c_char, c_int, c_void, CStr, CString},
    num::NonZero,
    ptr,
    sync::Arc,
};

use rusqlite::ffi;

#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Cell {
    pub fn sql(&self) -> String {
        match self {
            Cell::Null => "NULL".into(),
            Cell::Int(i) => i.to_string(),
            Cell::Real(x) => {
                let s = format!("{x:?}");
                if s.contains("inf") {
                    if *x > 0.0 { "9e999".into() } else { "-9e999".into() }
                } else {
                    s
                }
            },
            Cell::Text(t) => format!("'{}'", t.replace('\'', "''")),
            Cell::Blob(b) => format!("x'{}'", b.iter().map(|x| format!("{x:02x}")).collect::<String>()),
        }
    }
}

/// The error classes compared (messages are not).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Error,
    Busy,
    Locked,
    Constraint,
    Abort,
    ReadOnly,
    Corrupt,
    Full,
    Schema,
    Interrupt,
    Misuse,
    Range,
    TooBig,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ev {
    Ok,
    Row(Vec<Cell>),
    Done,
    Err(Class, String),
    /// The action named an empty slot: nothing ran (both engines alike).
    NoSlot,
    /// Turso panicked (caught); the engine is unusable afterwards.
    Panic(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ConnState {
    pub autocommit: bool,
    pub changes: i64,
    pub total_changes: i64,
    pub last_rowid: i64,
}

pub trait Engine {
    fn prepare(&mut self, slot: usize, conn: usize, sql: &str) -> Ev;
    fn bind(&mut self, slot: usize, idx: usize, v: &Cell) -> Ev;
    fn step(&mut self, slot: usize) -> Ev;
    fn reset(&mut self, slot: usize) -> Ev;
    fn finalize(&mut self, slot: usize);
    fn occupied(&self, slot: usize) -> bool;
    fn state(&self, conn: usize) -> ConnState;
    fn conns(&self) -> usize;
    /// The appvt module's hook calls since the last call (for traces).
    fn vt_events(&self) -> Vec<String>;
}

/// The placeholder for the engine's own ATTACH file in session SQL.
pub const AUX: &str = "$AUX";
/// Round 3: a second attached file (replaced before [`AUX`], its prefix).
pub const AUX2: &str = "$AUX2";

/// Session SQL for one engine: its own attached files.
fn subst(sql: &str, aux: &str, aux2: &str) -> String {
    sql.replace(AUX2, aux2).replace(AUX, aux)
}

// ------------------------------------------------------- app functions

/// The application functions both engines register.
pub const APP_FUNCTIONS: &[&str] = &["app_echo", "app_text", "app_after_text", "app_type", "app_concat", "app_count", "app_isum"];

// ------------------------------------------------------------- SQLite

pub struct Sqlite {
    conns: Vec<*mut ffi::sqlite3>,
    stmts: BTreeMap<usize, (usize, *mut ffi::sqlite3_stmt)>,
    vt: Arc<crate::vt::VtStore>,
    aux: String,
    aux2: String,
}

// SAFETY: one engine is used by one thread at a time.
unsafe impl Send for Sqlite {}

fn sqlite_class(rc: c_int) -> Class {
    match rc & 0xff {
        ffi::SQLITE_BUSY => Class::Busy,
        ffi::SQLITE_LOCKED => Class::Locked,
        ffi::SQLITE_CONSTRAINT => Class::Constraint,
        ffi::SQLITE_ABORT => Class::Abort,
        ffi::SQLITE_READONLY => Class::ReadOnly,
        ffi::SQLITE_CORRUPT | ffi::SQLITE_NOTADB => Class::Corrupt,
        ffi::SQLITE_FULL => Class::Full,
        ffi::SQLITE_SCHEMA => Class::Schema,
        ffi::SQLITE_INTERRUPT => Class::Interrupt,
        ffi::SQLITE_MISUSE => Class::Misuse,
        ffi::SQLITE_RANGE => Class::Range,
        ffi::SQLITE_TOOBIG => Class::TooBig,
        ffi::SQLITE_ERROR => Class::Error,
        _ => Class::Other,
    }
}

unsafe fn errmsg(db: *mut ffi::sqlite3) -> String {
    CStr::from_ptr(ffi::sqlite3_errmsg(db)).to_string_lossy().into_owned()
}

/// [`sqlite_value`] for the vtab module.
///
/// # Safety
/// `v` is a valid protected value.
pub unsafe fn sqlite_value_pub(v: *mut ffi::sqlite3_value) -> Cell {
    sqlite_value(v)
}

unsafe fn sqlite_value(v: *mut ffi::sqlite3_value) -> Cell {
    match ffi::sqlite3_value_type(v) {
        ffi::SQLITE_INTEGER => Cell::Int(ffi::sqlite3_value_int64(v)),
        ffi::SQLITE_FLOAT => Cell::Real(ffi::sqlite3_value_double(v)),
        ffi::SQLITE_TEXT => {
            let p = ffi::sqlite3_value_text(v);
            let n = ffi::sqlite3_value_bytes(v) as usize;
            Cell::Text(if p.is_null() { String::new() } else { String::from_utf8_lossy(std::slice::from_raw_parts(p, n)).into_owned() })
        },
        ffi::SQLITE_BLOB => {
            let p = ffi::sqlite3_value_blob(v) as *const u8;
            let n = ffi::sqlite3_value_bytes(v) as usize;
            Cell::Blob(if p.is_null() { Vec::new() } else { std::slice::from_raw_parts(p, n).to_vec() })
        },
        _ => Cell::Null,
    }
}

unsafe fn result_text(ctx: *mut ffi::sqlite3_context, s: &[u8]) {
    ffi::sqlite3_result_text(ctx, s.as_ptr() as *const c_char, s.len() as c_int, ffi::SQLITE_TRANSIENT());
}

/// app_echo(x): x itself (`sqlite3_result_value`).
unsafe extern "C" fn s_echo(ctx: *mut ffi::sqlite3_context, _n: c_int, argv: *mut *mut ffi::sqlite3_value) {
    ffi::sqlite3_result_value(ctx, *argv);
}

/// app_text(x): x read with `sqlite3_value_text`, returned as text; NULL stays NULL.
unsafe extern "C" fn s_text(ctx: *mut ffi::sqlite3_context, _n: c_int, argv: *mut *mut ffi::sqlite3_value) {
    let v = *argv;
    if ffi::sqlite3_value_type(v) == ffi::SQLITE_NULL {
        ffi::sqlite3_result_null(ctx);
        return;
    }
    let p = ffi::sqlite3_value_text(v);
    let n = ffi::sqlite3_value_bytes(v) as usize;
    if p.is_null() {
        result_text(ctx, b"");
    } else {
        result_text(ctx, std::slice::from_raw_parts(p, n));
    }
}

/// app_after_text(x): x read as text first, then x itself returned (the
/// value's type must not change by being read).
unsafe extern "C" fn s_after_text(ctx: *mut ffi::sqlite3_context, _n: c_int, argv: *mut *mut ffi::sqlite3_value) {
    let v = *argv;
    let _ = ffi::sqlite3_value_text(v);
    let _ = ffi::sqlite3_value_bytes(v);
    ffi::sqlite3_result_value(ctx, v);
}

/// app_type(x): `sqlite3_value_type` (1 int, 2 real, 3 text, 4 blob, 5 null).
unsafe extern "C" fn s_type(ctx: *mut ffi::sqlite3_context, _n: c_int, argv: *mut *mut ffi::sqlite3_value) {
    ffi::sqlite3_result_int64(ctx, ffi::sqlite3_value_type(*argv) as i64);
}

/// app_concat(x, y): both read as text and joined by '|' (NULL as '-').
unsafe extern "C" fn s_concat(ctx: *mut ffi::sqlite3_context, n: c_int, argv: *mut *mut ffi::sqlite3_value) {
    let mut out = Vec::new();
    for k in 0..n as usize {
        if k > 0 {
            out.push(b'|');
        }
        let v = *argv.add(k);
        if ffi::sqlite3_value_type(v) == ffi::SQLITE_NULL {
            out.push(b'-');
        } else {
            let p = ffi::sqlite3_value_text(v);
            let len = ffi::sqlite3_value_bytes(v) as usize;
            if !p.is_null() {
                out.extend_from_slice(std::slice::from_raw_parts(p, len));
            }
        }
    }
    result_text(ctx, &out);
}

/// app_count(x): the number of non-NULL x (aggregate and window).
unsafe extern "C" fn s_count_step(ctx: *mut ffi::sqlite3_context, _n: c_int, argv: *mut *mut ffi::sqlite3_value) {
    let acc = ffi::sqlite3_aggregate_context(ctx, 8) as *mut i64;
    if !acc.is_null() && ffi::sqlite3_value_type(*argv) != ffi::SQLITE_NULL {
        *acc += 1;
    }
}
unsafe extern "C" fn s_count_inverse(ctx: *mut ffi::sqlite3_context, _n: c_int, argv: *mut *mut ffi::sqlite3_value) {
    let acc = ffi::sqlite3_aggregate_context(ctx, 8) as *mut i64;
    if !acc.is_null() && ffi::sqlite3_value_type(*argv) != ffi::SQLITE_NULL {
        *acc -= 1;
    }
}
unsafe extern "C" fn s_count_value(ctx: *mut ffi::sqlite3_context) {
    let acc = ffi::sqlite3_aggregate_context(ctx, 0) as *mut i64;
    ffi::sqlite3_result_int64(ctx, if acc.is_null() { 0 } else { *acc });
}

/// app_isum(x): the sum of the INTEGER-typed x (wrapping), others ignored.
unsafe extern "C" fn s_isum_step(ctx: *mut ffi::sqlite3_context, _n: c_int, argv: *mut *mut ffi::sqlite3_value) {
    let acc = ffi::sqlite3_aggregate_context(ctx, 8) as *mut i64;
    if !acc.is_null() && ffi::sqlite3_value_type(*argv) == ffi::SQLITE_INTEGER {
        *acc = (*acc).wrapping_add(ffi::sqlite3_value_int64(*argv));
    }
}

pub(crate) unsafe fn register_sqlite(db: *mut ffi::sqlite3) {
    let det = ffi::SQLITE_UTF8 | ffi::SQLITE_DETERMINISTIC;
    type F = unsafe extern "C" fn(*mut ffi::sqlite3_context, c_int, *mut *mut ffi::sqlite3_value);
    let scalars: [(&str, c_int, F); 5] =
        [("app_echo", 1, s_echo), ("app_text", 1, s_text), ("app_after_text", 1, s_after_text), ("app_type", 1, s_type), ("app_concat", 2, s_concat)];
    for (name, n, f) in scalars {
        let c = CString::new(name).unwrap();
        let rc = ffi::sqlite3_create_function_v2(db, c.as_ptr(), n, det, ptr::null_mut(), Some(f), None, None, None);
        assert_eq!(rc, 0, "registering {name}");
    }
    let c = CString::new("app_count").unwrap();
    let rc = ffi::sqlite3_create_window_function(
        db,
        c.as_ptr(),
        1,
        det,
        ptr::null_mut(),
        Some(s_count_step),
        Some(s_count_value),
        Some(s_count_value),
        Some(s_count_inverse),
        None,
    );
    assert_eq!(rc, 0);
    let c = CString::new("app_isum").unwrap();
    let rc = ffi::sqlite3_create_function_v2(db, c.as_ptr(), 1, det, ptr::null_mut(), None, Some(s_isum_step), Some(s_count_value), None);
    assert_eq!(rc, 0);
}

impl Sqlite {
    /// `conns` connections on `path`, in WAL mode (Turso's only journal mode).
    pub fn open(path: &str, conns: usize, aux: &str, aux2: &str) -> Result<Sqlite, String> {
        let mut s = Sqlite { conns: Vec::new(), stmts: BTreeMap::new(), vt: Arc::default(), aux: aux.to_string(), aux2: aux2.to_string() };
        let p = CString::new(path).unwrap();
        for k in 0..conns {
            let mut db = ptr::null_mut();
            // SAFETY: valid out-pointer and NUL-terminated path.
            let rc = unsafe {
                ffi::sqlite3_open_v2(p.as_ptr(), &mut db, ffi::SQLITE_OPEN_READWRITE | ffi::SQLITE_OPEN_CREATE | ffi::SQLITE_OPEN_FULLMUTEX, ptr::null())
            };
            if rc != 0 {
                return Err(format!("sqlite open: rc {rc}"));
            }
            // SAFETY: db is open.
            unsafe {
                register_sqlite(db);
                crate::vt::register_sqlite(db, &s.vt);
            }
            s.conns.push(db);
            if k == 0 {
                // SAFETY: db is open; the statement is complete.
                let rc = unsafe { ffi::sqlite3_exec(db, c"PRAGMA journal_mode=WAL".as_ptr(), None, ptr::null_mut(), ptr::null_mut()) };
                if rc != 0 {
                    return Err("journal_mode=WAL".into());
                }
            }
        }
        Ok(s)
    }
}

impl Drop for Sqlite {
    fn drop(&mut self) {
        for (_, (_, st)) in std::mem::take(&mut self.stmts) {
            // SAFETY: each statement is finalized once.
            unsafe { ffi::sqlite3_finalize(st) };
        }
        for db in self.conns.drain(..) {
            // SAFETY: every statement is finalized.
            unsafe { ffi::sqlite3_close(db) };
        }
    }
}

impl Engine for Sqlite {
    fn conns(&self) -> usize {
        self.conns.len()
    }

    fn occupied(&self, slot: usize) -> bool {
        self.stmts.contains_key(&slot)
    }

    fn vt_events(&self) -> Vec<String> {
        self.vt.take_events()
    }

    fn prepare(&mut self, slot: usize, conn: usize, sql: &str) -> Ev {
        self.finalize(slot);
        let db = self.conns[conn];
        // Round 3 (MVCC sessions, one connection): SQLite has no BEGIN
        // CONCURRENT; on a single connection it is a deferred BEGIN.
        let mut q = subst(sql, &self.aux, &self.aux2);
        if q.starts_with("BEGIN CONCURRENT") {
            q = q.replacen("BEGIN CONCURRENT", "BEGIN DEFERRED", 1);
        }
        let c = CString::new(q).unwrap();
        let mut st = ptr::null_mut();
        // SAFETY: open db, NUL-terminated SQL, valid out-pointer.
        let rc = unsafe { ffi::sqlite3_prepare_v2(db, c.as_ptr(), -1, &mut st, ptr::null_mut()) };
        if rc != 0 {
            // SAFETY: open db.
            return Ev::Err(sqlite_class(rc), unsafe { errmsg(db) });
        }
        if st.is_null() {
            return Ev::Err(Class::Misuse, "empty statement".into());
        }
        self.stmts.insert(slot, (conn, st));
        Ev::Ok
    }

    fn bind(&mut self, slot: usize, idx: usize, v: &Cell) -> Ev {
        let Some(&(conn, st)) = self.stmts.get(&slot) else { return Ev::NoSlot };
        let i = idx as c_int;
        // SAFETY: live statement; SQLITE_TRANSIENT copies the buffers.
        let rc = unsafe {
            match v {
                Cell::Null => ffi::sqlite3_bind_null(st, i),
                Cell::Int(x) => ffi::sqlite3_bind_int64(st, i, *x),
                Cell::Real(x) => ffi::sqlite3_bind_double(st, i, *x),
                Cell::Text(t) => ffi::sqlite3_bind_text(st, i, t.as_ptr() as *const c_char, t.len() as c_int, ffi::SQLITE_TRANSIENT()),
                Cell::Blob(b) => ffi::sqlite3_bind_blob(st, i, b.as_ptr() as *const c_void, b.len() as c_int, ffi::SQLITE_TRANSIENT()),
            }
        };
        if rc != 0 {
            // SAFETY: open db.
            return Ev::Err(sqlite_class(rc), unsafe { errmsg(self.conns[conn]) });
        }
        Ev::Ok
    }

    fn step(&mut self, slot: usize) -> Ev {
        let Some(&(conn, st)) = self.stmts.get(&slot) else { return Ev::NoSlot };
        // SAFETY: live statement.
        unsafe {
            match ffi::sqlite3_step(st) {
                ffi::SQLITE_ROW => {
                    let n = ffi::sqlite3_column_count(st);
                    Ev::Row((0..n).map(|k| sqlite_value(ffi::sqlite3_column_value(st, k))).collect())
                },
                ffi::SQLITE_DONE => Ev::Done,
                rc => Ev::Err(sqlite_class(rc), errmsg(self.conns[conn])),
            }
        }
    }

    fn reset(&mut self, slot: usize) -> Ev {
        let Some(&(_, st)) = self.stmts.get(&slot) else { return Ev::NoSlot };
        // sqlite3_reset returns the last step's error again; the reset
        // itself always succeeds, so the result is not compared.
        // SAFETY: live statement.
        unsafe { ffi::sqlite3_reset(st) };
        Ev::Ok
    }

    fn finalize(&mut self, slot: usize) {
        if let Some((_, st)) = self.stmts.remove(&slot) {
            // SAFETY: removed from the table, finalized once.
            unsafe { ffi::sqlite3_finalize(st) };
        }
    }

    fn state(&self, conn: usize) -> ConnState {
        let db = self.conns[conn];
        // SAFETY: open db.
        unsafe {
            ConnState {
                autocommit: ffi::sqlite3_get_autocommit(db) != 0,
                changes: ffi::sqlite3_changes64(db),
                total_changes: ffi::sqlite3_total_changes64(db),
                last_rowid: ffi::sqlite3_last_insert_rowid(db),
            }
        }
    }
}

// -------------------------------------------------------------- Turso

use turso_core::{
    native_ext::{Aggregate, AggregateFunction, FunctionArity, ScalarCall, ScalarFunction},
    Connection, ConnectOpts, Database, ExternalFunc, IOResult, LimboError, OpenOptions, PlatformIO, Register, Statement, StepResult, Value,
};

type IOResultOr<T> = std::result::Result<IOResult<T>, Box<LimboError>>;

fn t_type(v: &Value) -> i64 {
    match v {
        Value::Null => 5,
        Value::Numeric(turso_core::Numeric::Integer(_)) => 1,
        Value::Numeric(_) => 2,
        Value::Text(_) => 3,
        Value::Blob(_) => 4,
    }
}

#[derive(Debug, Clone, Copy)]
struct TScalar(&'static str);

impl ScalarFunction for TScalar {
    type Call = TScalar;
    fn create_call(&self) -> turso_core::Result<TScalar> {
        Ok(*self)
    }
}

impl ScalarCall for TScalar {
    type Output = Value;
    fn step(&mut self, _c: &Arc<Connection>, args: &[Register]) -> IOResultOr<Value> {
        let v = args[0].get_value();
        let out = match self.0 {
            "app_echo" | "app_after_text" => v.clone(),
            "app_text" => match v.cast_text() {
                None => Value::Null,
                Some(s) => Value::from_text(s),
            },
            "app_type" => Value::from_i64(t_type(v)),
            "app_concat" => {
                let parts: Vec<String> = args.iter().map(|a| a.get_value().cast_text().unwrap_or_else(|| "-".into())).collect();
                Value::from_text(parts.join("|"))
            },
            _ => unreachable!(),
        };
        Ok(IOResult::Done(out))
    }
}

#[derive(Debug, Clone, Copy)]
struct TAgg(&'static str);

#[derive(Debug)]
struct TAcc(&'static str, i64);

impl AggregateFunction for TAgg {
    type Accumulator = TAcc;
    fn create_accumulator(&self) -> turso_core::Result<TAcc> {
        Ok(TAcc(self.0, 0))
    }
}

impl Aggregate for TAcc {
    fn step(&mut self, args: &[Value]) -> IOResultOr<()> {
        match (self.0, &args[0]) {
            (_, Value::Null) => {},
            ("app_count", _) => self.1 += 1,
            ("app_isum", Value::Numeric(turso_core::Numeric::Integer(i))) => self.1 = self.1.wrapping_add(*i),
            _ => {},
        }
        Ok(IOResult::Done(()))
    }
    fn finalize(&mut self) -> IOResultOr<Value> {
        Ok(IOResult::Done(Value::from_i64(self.1)))
    }
}

pub fn turso_class(e: &LimboError) -> Class {
    match e {
        LimboError::Corrupt(..) | LimboError::NotADB => Class::Corrupt,
        LimboError::Constraint(_) | LimboError::ConstraintViolation(_) | LimboError::ForeignKeyConstraint(_) | LimboError::Raise(..) => {
            Class::Constraint
        },
        LimboError::DatabaseFull | LimboError::SequenceExhausted { .. } => Class::Full,
        LimboError::TableLocked => Class::Locked,
        LimboError::ReadOnly => Class::ReadOnly,
        LimboError::Busy | LimboError::BusySnapshot | LimboError::StatementsInProgress(_) => Class::Busy,
        LimboError::BlobHandleExpired => Class::Abort,
        LimboError::SchemaUpdated | LimboError::SchemaConflict => Class::Schema,
        LimboError::Interrupt => Class::Interrupt,
        LimboError::TooBig => Class::TooBig,
        _ => Class::Error,
    }
}

pub struct Turso {
    pub db: Arc<Database>,
    pub conns: Vec<Arc<Connection>>,
    stmts: BTreeMap<usize, (usize, Statement)>,
    vt: Arc<crate::vt::VtStore>,
    aux: String,
    aux2: String,
}

/// Steps a statement through IO until a row, done or error. Panics are
/// caught by the caller.
fn turso_step(st: &mut Statement) -> Ev {
    for _ in 0..1_000_000 {
        match st.step() {
            Ok(StepResult::Row) => {
                let row = st.row().expect("row after StepResult::Row");
                return Ev::Row(
                    row.get_values()
                        .map(|v| match v {
                            Value::Null => Cell::Null,
                            Value::Numeric(turso_core::Numeric::Integer(i)) => Cell::Int(*i),
                            Value::Numeric(n) => Cell::Real(n.to_f64()),
                            Value::Text(t) => Cell::Text(t.as_str().to_string()),
                            Value::Blob(b) => Cell::Blob(b.to_vec()),
                        })
                        .collect(),
                );
            },
            Ok(StepResult::Done) => return Ev::Done,
            Ok(StepResult::IO | StepResult::Yield | StepResult::Sleep { .. }) => {
                if let Err(e) = st._io().step() {
                    return Ev::Err(turso_class(&e), format!("io: {e}"));
                }
            },
            Ok(StepResult::Busy) => return Ev::Err(Class::Busy, "StepResult::Busy".into()),
            Ok(StepResult::Interrupt) => return Ev::Err(Class::Interrupt, "StepResult::Interrupt".into()),
            Err(e) => return Ev::Err(turso_class(&e), e.to_string()),
        }
    }
    Ev::Err(Class::Other, "harness: statement did not finish in 1e6 IO steps".into())
}

impl Turso {
    pub fn open(path: &str, conns: usize, aux: &str, aux2: &str, mvcc: bool) -> Result<Turso, String> {
        let vt: Arc<crate::vt::VtStore> = Arc::default();
        let mut opts = OpenOptions::new(Arc::new(turso_sqlite::SqliteDialect))
            .db_opts(turso_core::DatabaseOpts::new().with_attach(true).with_mvcc(mvcc))
            .native_module(crate::vt::MODULE, turso_core::native_ext::VTabKind::VirtualTable, crate::vt::TModule(vt.clone()));
        for name in ["app_echo", "app_text", "app_after_text", "app_type"] {
            opts = opts.extension_function(ExternalFunc::new_native_scalar(name.into(), FunctionArity::Exact(1), true, TScalar(name)).map_err(|e| e.to_string())?);
        }
        opts = opts.extension_function(ExternalFunc::new_native_scalar("app_concat".into(), FunctionArity::Exact(2), true, TScalar("app_concat")).map_err(|e| e.to_string())?);
        for name in ["app_count", "app_isum"] {
            opts = opts.extension_function(ExternalFunc::new_native_aggregate(name.into(), 1, TAgg(name)).map_err(|e| e.to_string())?);
        }
        let io = Arc::new(PlatformIO::new().map_err(|e| e.to_string())?);
        let db = Database::open(io, path, opts).map_err(|e| format!("turso open: {e}"))?;
        let mut t = Turso { db: db.clone(), conns: Vec::new(), stmts: BTreeMap::new(), vt, aux: aux.to_string(), aux2: aux2.to_string() };
        for _ in 0..conns {
            t.conns.push(db.connect(ConnectOpts::new()).map_err(|e| format!("turso connect: {e}"))?);
        }
        Ok(t)
    }

    pub fn close(&mut self) {
        self.stmts.clear();
        for c in self.conns.drain(..) {
            let _ = c.close();
        }
    }
}

impl Engine for Turso {
    fn conns(&self) -> usize {
        self.conns.len()
    }

    fn occupied(&self, slot: usize) -> bool {
        self.stmts.contains_key(&slot)
    }

    fn vt_events(&self) -> Vec<String> {
        self.vt.take_events()
    }

    fn prepare(&mut self, slot: usize, conn: usize, sql: &str) -> Ev {
        self.finalize(slot);
        match self.conns[conn].prepare(subst(sql, &self.aux, &self.aux2)) {
            Ok(st) => {
                self.stmts.insert(slot, (conn, st));
                Ev::Ok
            },
            Err(e) => Ev::Err(turso_class(&e), e.to_string()),
        }
    }

    fn bind(&mut self, slot: usize, idx: usize, v: &Cell) -> Ev {
        let Some((_, st)) = self.stmts.get_mut(&slot) else { return Ev::NoSlot };
        let val = match v {
            Cell::Null => Value::Null,
            Cell::Int(i) => Value::from_i64(*i),
            Cell::Real(x) => Value::from_f64(*x),
            Cell::Text(t) => Value::from_text(t.clone()),
            Cell::Blob(b) => Value::from_blob(b.clone().into()),
        };
        match st.bind_at(NonZero::new(idx).unwrap(), val) {
            Ok(()) => Ev::Ok,
            Err(e) => Ev::Err(turso_class(&e), e.to_string()),
        }
    }

    fn step(&mut self, slot: usize) -> Ev {
        let Some((_, st)) = self.stmts.get_mut(&slot) else { return Ev::NoSlot };
        turso_step(st)
    }

    fn reset(&mut self, slot: usize) -> Ev {
        let Some((_, st)) = self.stmts.get_mut(&slot) else { return Ev::NoSlot };
        match st.reset() {
            Ok(()) => Ev::Ok,
            // Recorded but compared like SQLite's (always Ok): see README.md.
            Err(e) => Ev::Err(turso_class(&e), format!("reset: {e}")),
        }
    }

    fn finalize(&mut self, slot: usize) {
        self.stmts.remove(&slot);
    }

    fn state(&self, conn: usize) -> ConnState {
        let c = &self.conns[conn];
        ConnState { autocommit: c.get_auto_commit(), changes: c.changes(), total_changes: c.total_changes(), last_rowid: c.last_insert_rowid() }
    }
}
