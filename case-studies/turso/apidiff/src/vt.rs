// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A writable application virtual table, `appvt`, registered identically in
//! both engines (round 2): SQLite through `sqlite3_create_module_v2` with an
//! iVersion-1 `sqlite3_module` (xBegin/xSync/xCommit/xRollback, no
//! savepoint methods), Turso through its native API
//! (`OpenOptions::native_module`, `VirtualTableModule` / `VirtualTable` /
//! `TableUpdate`, which has begin/commit/rollback and no savepoint hooks).
//! Both sides share the module logic below ([`VtStore`]), so any difference
//! comes from when and how the engine calls the module.
//!
//! `CREATE VIRTUAL TABLE vtN USING appvt(vtN)`: the argument names the
//! backing storage (Turso's `create` does not receive the table name).
//! Columns `x(a, b)`; full scans in rowid order (no constraints used); the
//! cursor reads a copy of the rows taken at xFilter.
//!
//! Writes (xUpdate / TableUpdate::step), single row:
//! - INSERT: rowid = the given INTEGER or max+1; a given rowid that exists
//!   fails before writing; b = 'boom' fails before writing; a = 13 writes
//!   the row and then fails ("write then fail").
//! - UPDATE: old row replaced by the new one (rowid may change; a new rowid
//!   that exists fails before writing); a = 13 writes, then fails.
//! - DELETE: removes the row; a removed row with a = 99 fails after the
//!   removal.
//!
//! Transactions: xBegin copies the rows (the first time in a transaction),
//! xCommit drops the copy, xRollback restores it. So a statement that
//! fails after writing keeps its write until the transaction rolls back,
//! in both engines (SQLite has no statement rollback for an iVersion-1
//! module). Every hook call is recorded in an event log for traces.

use std::{
    collections::BTreeMap,
    ffi::{c_char, c_int, c_void, CStr, CString},
    ptr,
    sync::{Arc, Mutex},
};

use rusqlite::ffi;

use crate::engines::Cell;

pub const MODULE: &str = "appvt";
const SCHEMA: &str = "CREATE TABLE x(a, b)";

type Rows = BTreeMap<i64, (Cell, Cell)>;

#[derive(Default)]
struct VtData {
    rows: Rows,
    before: Option<Rows>,
}

#[derive(Default)]
pub struct VtStore {
    tables: Mutex<BTreeMap<String, VtData>>,
    events: Mutex<Vec<String>>,
}

impl VtStore {
    fn event(&self, name: &str, e: impl Into<String>) {
        self.events.lock().unwrap().push(format!("{name}:{}", e.into()));
    }

    pub fn take_events(&self) -> Vec<String> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }

    fn scan(&self, name: &str) -> Vec<(i64, Cell, Cell)> {
        let t = self.tables.lock().unwrap();
        t.get(name).map_or_else(Vec::new, |d| d.rows.iter().map(|(k, (a, b))| (*k, a.clone(), b.clone())).collect())
    }

    fn begin(&self, name: &str) {
        let mut t = self.tables.lock().unwrap();
        let d = t.entry(name.to_string()).or_default();
        if d.before.is_none() {
            d.before = Some(d.rows.clone());
            drop(t);
            self.event(name, "begin");
        } else {
            drop(t);
            self.event(name, "begin(again)");
        }
    }

    fn commit(&self, name: &str) {
        let mut t = self.tables.lock().unwrap();
        let had = t.get_mut(name).and_then(|d| d.before.take()).is_some();
        drop(t);
        self.event(name, if had { "commit" } else { "commit(no begin)" });
    }

    fn rollback(&self, name: &str) {
        let mut t = self.tables.lock().unwrap();
        let mut had = false;
        if let Some(d) = t.get_mut(name) {
            if let Some(b) = d.before.take() {
                d.rows = b;
                had = true;
            }
        }
        drop(t);
        self.event(name, if had { "rollback" } else { "rollback(no begin)" });
    }

    fn destroy(&self, name: &str) {
        self.tables.lock().unwrap().remove(name);
        self.event(name, "destroy");
    }

    /// Round 3: xRename / VirtualTable::rename. The storage keeps its name
    /// (the module argument), only the call is logged.
    fn rename(&self, name: &str, to: &str) {
        self.event(name, format!("rename {to}"));
    }

    /// One xUpdate. `old`: the rowid replaced or deleted (None for an
    /// insert); `new_rowid`: the rowid given for the new row (NULL: pick);
    /// `cols`: the new row (None for a delete). Ok(rowid of an inserted row).
    fn update(&self, name: &str, old: Option<i64>, new_rowid: &Cell, cols: Option<(Cell, Cell)>) -> Result<Option<i64>, String> {
        let mut t = self.tables.lock().unwrap();
        let d = t.entry(name.to_string()).or_default();
        let Some((a, b)) = cols else {
            let old = old.ok_or("vt: delete without a rowid")?;
            let removed = d.rows.remove(&old);
            drop(t);
            self.event(name, format!("delete {old}"));
            if matches!(removed, Some((Cell::Int(99), _))) {
                return Err("vt: deleted a = 99, then failed".into());
            }
            return Ok(None);
        };
        if b == Cell::Text("boom".into()) {
            return Err("vt: b = 'boom' refused".into());
        }
        let rowid = match new_rowid {
            Cell::Int(i) => *i,
            Cell::Null => match old {
                Some(o) => o,
                None => d.rows.keys().next_back().map_or(1, |m| m.wrapping_add(1)),
            },
            other => return Err(format!("vt: rowid {other:?} is not an integer")),
        };
        if Some(rowid) != old && d.rows.contains_key(&rowid) {
            return Err(format!("vt: rowid {rowid} exists"));
        }
        if let Some(o) = old {
            d.rows.remove(&o);
        }
        let fail = a == Cell::Int(13);
        d.rows.insert(rowid, (a, b));
        drop(t);
        self.event(name, format!("{} {rowid}", if old.is_some() { "update" } else { "insert" }));
        if fail {
            return Err("vt: wrote a = 13, then failed".into());
        }
        Ok(if old.is_none() { Some(rowid) } else { None })
    }
}

// ------------------------------------------------------------- SQLite

#[repr(C)]
struct SVtab {
    base: ffi::sqlite3_vtab,
    store: Arc<VtStore>,
    name: String,
}

#[repr(C)]
struct SCursor {
    base: ffi::sqlite3_vtab_cursor,
    rows: Vec<(i64, Cell, Cell)>,
    pos: usize,
}

unsafe fn set_err(vtab: *mut ffi::sqlite3_vtab, msg: &str) {
    if !(*vtab).zErrMsg.is_null() {
        ffi::sqlite3_free((*vtab).zErrMsg as *mut c_void);
    }
    let c = CString::new(msg.replace('\0', "")).unwrap();
    (*vtab).zErrMsg = ffi::sqlite3_mprintf(c"%s".as_ptr(), c.as_ptr());
}

unsafe fn s_value(v: *mut ffi::sqlite3_value) -> Cell {
    crate::engines::sqlite_value_pub(v)
}

unsafe extern "C" fn x_create(
    db: *mut ffi::sqlite3,
    aux: *mut c_void,
    argc: c_int,
    argv: *const *const c_char,
    pp: *mut *mut ffi::sqlite3_vtab,
    _err: *mut *mut c_char,
) -> c_int {
    let store = &*(aux as *const Arc<VtStore>);
    // argv: module, database, table, args...
    let name = if argc > 3 {
        CStr::from_ptr(*argv.add(3)).to_string_lossy().trim().trim_matches(|c| c == '\'' || c == '"').to_string()
    } else {
        CStr::from_ptr(*argv.add(2)).to_string_lossy().into_owned()
    };
    let schema = CString::new(SCHEMA).unwrap();
    let rc = ffi::sqlite3_declare_vtab(db, schema.as_ptr());
    if rc != 0 {
        return rc;
    }
    // Round 3: innocuous on both sides (Turso's module says so too), so
    // triggers and views may read it whatever trusted_schema says.
    ffi::sqlite3_vtab_config(db, ffi::SQLITE_VTAB_INNOCUOUS);
    let v = Box::new(SVtab { base: ffi::sqlite3_vtab { pModule: ptr::null(), nRef: 0, zErrMsg: ptr::null_mut() }, store: store.clone(), name });
    *pp = Box::into_raw(v) as *mut ffi::sqlite3_vtab;
    0
}

unsafe extern "C" fn x_best_index(_v: *mut ffi::sqlite3_vtab, info: *mut ffi::sqlite3_index_info) -> c_int {
    (*info).idxNum = 0;
    (*info).estimatedCost = 1_000_000.0;
    (*info).estimatedRows = 1000;
    0
}

unsafe extern "C" fn x_disconnect(v: *mut ffi::sqlite3_vtab) -> c_int {
    let b = Box::from_raw(v as *mut SVtab);
    if !b.base.zErrMsg.is_null() {
        ffi::sqlite3_free(b.base.zErrMsg as *mut c_void);
    }
    drop(b);
    0
}

unsafe extern "C" fn x_destroy(v: *mut ffi::sqlite3_vtab) -> c_int {
    let s = &*(v as *mut SVtab);
    s.store.destroy(&s.name);
    x_disconnect(v)
}

unsafe extern "C" fn x_rename(v: *mut ffi::sqlite3_vtab, to: *const c_char) -> c_int {
    let s = &*(v as *mut SVtab);
    s.store.rename(&s.name, &CStr::from_ptr(to).to_string_lossy());
    0
}

unsafe extern "C" fn x_open(_v: *mut ffi::sqlite3_vtab, pp: *mut *mut ffi::sqlite3_vtab_cursor) -> c_int {
    let c = Box::new(SCursor { base: ffi::sqlite3_vtab_cursor { pVtab: ptr::null_mut() }, rows: Vec::new(), pos: 0 });
    *pp = Box::into_raw(c) as *mut ffi::sqlite3_vtab_cursor;
    0
}

unsafe extern "C" fn x_close(c: *mut ffi::sqlite3_vtab_cursor) -> c_int {
    drop(Box::from_raw(c as *mut SCursor));
    0
}

unsafe extern "C" fn x_filter(c: *mut ffi::sqlite3_vtab_cursor, _n: c_int, _s: *const c_char, _argc: c_int, _argv: *mut *mut ffi::sqlite3_value) -> c_int {
    let cur = &mut *(c as *mut SCursor);
    let v = &*((*c).pVtab as *mut SVtab);
    cur.rows = v.store.scan(&v.name);
    cur.pos = 0;
    0
}

unsafe extern "C" fn x_next(c: *mut ffi::sqlite3_vtab_cursor) -> c_int {
    (*(c as *mut SCursor)).pos += 1;
    0
}

unsafe extern "C" fn x_eof(c: *mut ffi::sqlite3_vtab_cursor) -> c_int {
    let cur = &*(c as *mut SCursor);
    c_int::from(cur.pos >= cur.rows.len())
}

unsafe extern "C" fn x_column(c: *mut ffi::sqlite3_vtab_cursor, ctx: *mut ffi::sqlite3_context, i: c_int) -> c_int {
    let cur = &*(c as *mut SCursor);
    let r = &cur.rows[cur.pos];
    let v = if i == 0 { &r.1 } else { &r.2 };
    match v {
        Cell::Null => ffi::sqlite3_result_null(ctx),
        Cell::Int(x) => ffi::sqlite3_result_int64(ctx, *x),
        Cell::Real(x) => ffi::sqlite3_result_double(ctx, *x),
        Cell::Text(t) => ffi::sqlite3_result_text(ctx, t.as_ptr() as *const c_char, t.len() as c_int, ffi::SQLITE_TRANSIENT()),
        Cell::Blob(b) => ffi::sqlite3_result_blob(ctx, b.as_ptr() as *const c_void, b.len() as c_int, ffi::SQLITE_TRANSIENT()),
    }
    0
}

unsafe extern "C" fn x_rowid(c: *mut ffi::sqlite3_vtab_cursor, out: *mut ffi::sqlite3_int64) -> c_int {
    let cur = &*(c as *mut SCursor);
    *out = cur.rows[cur.pos].0;
    0
}

unsafe extern "C" fn x_update(v: *mut ffi::sqlite3_vtab, argc: c_int, argv: *mut *mut ffi::sqlite3_value, out: *mut ffi::sqlite3_int64) -> c_int {
    let s = &*(v as *mut SVtab);
    let arg = |k: usize| s_value(*argv.add(k));
    let r = if argc == 1 {
        match arg(0) {
            Cell::Int(o) => s.store.update(&s.name, Some(o), &Cell::Null, None),
            other => Err(format!("vt: delete of rowid {other:?}")),
        }
    } else {
        let old = match arg(0) {
            Cell::Null => None,
            Cell::Int(o) => Some(o),
            other => {
                set_err(v, &format!("vt: old rowid {other:?}"));
                return ffi::SQLITE_ERROR;
            },
        };
        let cols = (if argc > 2 { arg(2) } else { Cell::Null }, if argc > 3 { arg(3) } else { Cell::Null });
        s.store.update(&s.name, old, &arg(1), Some(cols))
    };
    match r {
        Ok(Some(rowid)) => {
            *out = rowid;
            0
        },
        Ok(None) => 0,
        Err(m) => {
            set_err(v, &m);
            ffi::SQLITE_ERROR
        },
    }
}

unsafe extern "C" fn x_begin(v: *mut ffi::sqlite3_vtab) -> c_int {
    let s = &*(v as *mut SVtab);
    s.store.begin(&s.name);
    0
}
unsafe extern "C" fn x_sync(_v: *mut ffi::sqlite3_vtab) -> c_int {
    0
}
unsafe extern "C" fn x_commit(v: *mut ffi::sqlite3_vtab) -> c_int {
    let s = &*(v as *mut SVtab);
    s.store.commit(&s.name);
    0
}
unsafe extern "C" fn x_rollback(v: *mut ffi::sqlite3_vtab) -> c_int {
    let s = &*(v as *mut SVtab);
    s.store.rollback(&s.name);
    0
}

static S_MODULE: ffi::sqlite3_module = ffi::sqlite3_module {
    iVersion: 1,
    xCreate: Some(x_create),
    xConnect: Some(x_create),
    xBestIndex: Some(x_best_index),
    xDisconnect: Some(x_disconnect),
    xDestroy: Some(x_destroy),
    xOpen: Some(x_open),
    xClose: Some(x_close),
    xFilter: Some(x_filter),
    xNext: Some(x_next),
    xEof: Some(x_eof),
    xColumn: Some(x_column),
    xRowid: Some(x_rowid),
    xUpdate: Some(x_update),
    xBegin: Some(x_begin),
    xSync: Some(x_sync),
    xCommit: Some(x_commit),
    xRollback: Some(x_rollback),
    xFindFunction: None,
    xRename: Some(x_rename),
    xSavepoint: None,
    xRelease: None,
    xRollbackTo: None,
    xShadowName: None,
    xIntegrity: None,
};

unsafe extern "C" fn drop_store(p: *mut c_void) {
    drop(Box::from_raw(p as *mut Arc<VtStore>));
}

/// Registers `appvt` on a SQLite connection, backed by `store`.
///
/// # Safety
/// `db` is an open connection.
pub unsafe fn register_sqlite(db: *mut ffi::sqlite3, store: &Arc<VtStore>) {
    let aux = Box::into_raw(Box::new(store.clone())) as *mut c_void;
    let name = CString::new(MODULE).unwrap();
    let rc = ffi::sqlite3_create_module_v2(db, name.as_ptr(), &S_MODULE, aux, Some(drop_store));
    assert_eq!(rc, 0, "registering {MODULE}");
}

// -------------------------------------------------------------- Turso

use turso_core::{
    native_ext::{ConstraintInfo, ConstraintUsage, IndexInfo, OrderByInfo, TableUpdate, VirtualTable, VirtualTableCursor, VirtualTableModule},
    Connection, IOResult, LimboError, Value,
};

type IOResultOr<T> = std::result::Result<IOResult<T>, Box<LimboError>>;

fn t_cell(v: &Value) -> Cell {
    match v {
        Value::Null => Cell::Null,
        Value::Numeric(turso_core::Numeric::Integer(i)) => Cell::Int(*i),
        Value::Numeric(n) => Cell::Real(n.to_f64()),
        Value::Text(t) => Cell::Text(t.as_str().to_string()),
        Value::Blob(b) => Cell::Blob(b.to_vec()),
    }
}

fn t_value(c: &Cell) -> Value {
    match c {
        Cell::Null => Value::Null,
        Cell::Int(i) => Value::from_i64(*i),
        Cell::Real(x) => Value::from_f64(*x),
        Cell::Text(t) => Value::from_text(t.clone()),
        Cell::Blob(b) => Value::from_blob(b.clone().into()),
    }
}

#[derive(Debug)]
pub struct TModule(pub Arc<VtStore>);

impl std::fmt::Debug for VtStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VtStore")
    }
}

#[derive(Debug)]
pub struct TTable {
    store: Arc<VtStore>,
    name: String,
}

impl VirtualTableModule for TModule {
    type Table = TTable;
    fn schema(&self, _args: &[Value]) -> turso_core::Result<String> {
        Ok(SCHEMA.into())
    }
    fn create(&self, args: &[Value]) -> turso_core::Result<TTable> {
        let name = match args.first() {
            Some(v) => match t_cell(v) {
                Cell::Text(t) => t.trim().trim_matches(|c| c == '\'' || c == '"').to_string(),
                other => other.sql(),
            },
            None => String::from("(unnamed)"),
        };
        Ok(TTable { store: self.0.clone(), name })
    }
    fn innocuous(&self) -> bool {
        true
    }
}

impl VirtualTable for TTable {
    type Cursor = TCursor;
    fn open(&self, _c: Arc<Connection>) -> turso_core::Result<TCursor> {
        Ok(TCursor { store: self.store.clone(), name: self.name.clone(), rows: Vec::new(), pos: 0 })
    }
    fn best_index(&self, constraints: &[ConstraintInfo], _o: &[OrderByInfo]) -> Result<IndexInfo, turso_ext::ResultCode> {
        Ok(IndexInfo {
            estimated_cost: 1_000_000.0,
            estimated_rows: 1000,
            constraint_usages: constraints.iter().map(|_| ConstraintUsage { argv_index: None, omit: false }).collect(),
            ..Default::default()
        })
    }
    fn readonly(&self) -> bool {
        false
    }
    fn create_update(&mut self, args: Vec<Value>) -> turso_core::Result<Box<dyn TableUpdate>> {
        Ok(Box::new(TUpdate { store: self.store.clone(), name: self.name.clone(), args }))
    }
    fn begin(&mut self) -> turso_core::Result<()> {
        self.store.begin(&self.name);
        Ok(())
    }
    fn commit(&mut self) -> turso_core::Result<()> {
        self.store.commit(&self.name);
        Ok(())
    }
    fn rollback(&mut self) -> turso_core::Result<()> {
        self.store.rollback(&self.name);
        Ok(())
    }
    fn destroy(&mut self) -> turso_core::Result<()> {
        self.store.destroy(&self.name);
        Ok(())
    }
    fn rename(&mut self, to: &str) -> turso_core::Result<()> {
        self.store.rename(&self.name, to);
        Ok(())
    }
}

pub struct TCursor {
    store: Arc<VtStore>,
    name: String,
    rows: Vec<(i64, Cell, Cell)>,
    pos: usize,
}

impl VirtualTableCursor for TCursor {
    fn filter(&mut self, _args: &[Value], _s: Option<&str>, _n: i32) -> IOResultOr<bool> {
        self.rows = self.store.scan(&self.name);
        self.pos = 0;
        Ok(IOResult::Done(!self.rows.is_empty()))
    }
    fn next(&mut self) -> IOResultOr<bool> {
        self.pos += 1;
        Ok(IOResult::Done(self.pos < self.rows.len()))
    }
    fn column(&mut self, column: usize) -> IOResultOr<Value> {
        let r = &self.rows[self.pos];
        Ok(IOResult::Done(t_value(if column == 0 { &r.1 } else { &r.2 })))
    }
    fn rowid(&self) -> i64 {
        self.rows[self.pos].0
    }
}

struct TUpdate {
    store: Arc<VtStore>,
    name: String,
    args: Vec<Value>,
}

impl TableUpdate for TUpdate {
    fn step(&mut self) -> IOResultOr<Option<i64>> {
        let a: Vec<Cell> = self.args.iter().map(t_cell).collect();
        // Turso: delete = [rowid, NULL]; insert = [NULL, rowid?, cols..];
        // update = [old rowid, new rowid, cols..].
        let r = if a.len() == 2 {
            match &a[0] {
                Cell::Int(o) => self.store.update(&self.name, Some(*o), &Cell::Null, None),
                other => Err(format!("vt: delete of rowid {other:?}")),
            }
        } else {
            let old = match &a[0] {
                Cell::Null => Ok(None),
                Cell::Int(o) => Ok(Some(*o)),
                other => Err(format!("vt: old rowid {other:?}")),
            };
            match old {
                Ok(old) => self.store.update(&self.name, old, &a[1], Some((a.get(2).cloned().unwrap_or(Cell::Null), a.get(3).cloned().unwrap_or(Cell::Null)))),
                Err(e) => Err(e),
            }
        };
        r.map(IOResult::Done).map_err(|m| Box::new(LimboError::ExtensionError(m)))
    }
}
