// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `memvt`: a minimal writable virtual table written with turso_ext, the
//! documented Turso extension API (extensions/core/README.md).
//! `CREATE VIRTUAL TABLE v USING memvt` has columns (a, b); rows are kept in
//! memory (one store per process, shared by every connection, like a module
//! that keeps its data outside the database file). begin() snapshots the
//! rows, commit() drops the snapshot, rollback() restores it. Every hook
//! call is printed to stderr as `[memvt] ...`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use turso_ext::{
    register_extension, Connection, ResultCode, VTabCursor, VTabKind, VTabModule,
    VTabModuleDerive, VTable, Value, ValueType,
};

register_extension! {
    vtabs: { MemVtModule },
}

#[derive(Clone, Debug)]
enum V {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
}

impl V {
    fn from(v: &Value) -> V {
        match v.value_type() {
            ValueType::Integer => V::Int(v.to_integer().unwrap_or(0)),
            ValueType::Float => V::Real(v.to_float().unwrap_or(0.0)),
            ValueType::Text => V::Text(v.to_text().unwrap_or("").to_string()),
            _ => V::Null,
        }
    }
    fn to(&self) -> Value {
        match self {
            V::Null => Value::null(),
            V::Int(i) => Value::from_integer(*i),
            V::Real(f) => Value::from_float(*f),
            V::Text(s) => Value::from_text(s.clone()),
        }
    }
}

type Rows = BTreeMap<i64, (V, V)>;

struct Store {
    rows: Rows,
    before: Option<Rows>,
}

static STORE: Mutex<Store> = Mutex::new(Store { rows: BTreeMap::new(), before: None });

fn log(s: &str) {
    eprintln!("  [memvt] {s}");
}

#[derive(VTabModuleDerive, Default)]
pub struct MemVtModule;

impl VTabModule for MemVtModule {
    type Table = MemVtTable;
    const VTAB_KIND: VTabKind = VTabKind::VirtualTable;
    const NAME: &'static str = "memvt";
    const READONLY: bool = false;

    fn create(_args: &[Value]) -> Result<(String, Self::Table), ResultCode> {
        log("create");
        Ok(("CREATE TABLE x(a, b)".into(), MemVtTable))
    }
}

pub struct MemVtTable;

pub struct MemVtCursor {
    rows: Vec<(i64, V, V)>,
    pos: usize,
}

impl VTable for MemVtTable {
    type Cursor = MemVtCursor;
    type Error = String;

    fn open(&self, _conn: Option<Arc<Connection>>) -> Result<Self::Cursor, Self::Error> {
        Ok(MemVtCursor { rows: Vec::new(), pos: 0 })
    }
    fn begin(&mut self) -> Result<(), Self::Error> {
        log("begin");
        let mut s = STORE.lock().unwrap();
        s.before = Some(s.rows.clone());
        Ok(())
    }
    fn commit(&mut self) -> Result<(), Self::Error> {
        log("commit");
        STORE.lock().unwrap().before = None;
        Ok(())
    }
    fn rollback(&mut self) -> Result<(), Self::Error> {
        log("rollback");
        let mut s = STORE.lock().unwrap();
        if let Some(b) = s.before.take() {
            s.rows = b;
        }
        Ok(())
    }
    fn insert(&mut self, args: &[Value]) -> Result<i64, Self::Error> {
        let mut s = STORE.lock().unwrap();
        let rowid = s.rows.keys().next_back().copied().unwrap_or(0) + 1;
        let a = args.first().map(V::from).unwrap_or(V::Null);
        let b = args.get(1).map(V::from).unwrap_or(V::Null);
        log(&format!("insert rowid={rowid} a={a:?} b={b:?}"));
        s.rows.insert(rowid, (a, b));
        Ok(rowid)
    }
    fn update(&mut self, rowid: i64, args: &[Value]) -> Result<(), Self::Error> {
        let a = args.first().map(V::from).unwrap_or(V::Null);
        let b = args.get(1).map(V::from).unwrap_or(V::Null);
        log(&format!("update rowid={rowid} a={a:?} b={b:?}"));
        STORE.lock().unwrap().rows.insert(rowid, (a, b));
        Ok(())
    }
    fn delete(&mut self, rowid: i64) -> Result<(), Self::Error> {
        log(&format!("delete rowid={rowid}"));
        STORE.lock().unwrap().rows.remove(&rowid);
        Ok(())
    }
    fn destroy(&mut self) -> Result<(), Self::Error> {
        log("destroy");
        Ok(())
    }
}

impl VTabCursor for MemVtCursor {
    type Error = String;
    fn filter(&mut self, _args: &[Value], _idx: Option<(&str, i32)>) -> ResultCode {
        self.rows = STORE
            .lock()
            .unwrap()
            .rows
            .iter()
            .map(|(k, (a, b))| (*k, a.clone(), b.clone()))
            .collect();
        self.pos = 0;
        if self.rows.is_empty() {
            ResultCode::EOF
        } else {
            ResultCode::OK
        }
    }
    fn rowid(&self) -> i64 {
        self.rows.get(self.pos).map(|r| r.0).unwrap_or(-1)
    }
    fn column(&self, idx: u32) -> Result<Value, Self::Error> {
        let r = self.rows.get(self.pos).ok_or("out of range")?;
        match idx {
            0 => Ok(r.1.to()),
            1 => Ok(r.2.to()),
            _ => Err("bad column".into()),
        }
    }
    fn eof(&self) -> bool {
        self.pos >= self.rows.len()
    }
    fn next(&mut self) -> ResultCode {
        self.pos += 1;
        if self.eof() {
            ResultCode::EOF
        } else {
            ResultCode::OK
        }
    }
}
