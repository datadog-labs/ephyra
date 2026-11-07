//! SQLite 3.50.4 side of repro.rs (the code in posted.md) (raw sqlite3_* calls through
//! rusqlite::ffi so the reset result can be printed), printing the same lines
//! plus the final count.
use rusqlite::{ffi, Connection};
use std::ffi::CString;

unsafe fn prep(c: &Connection, sql: &str) -> *mut ffi::sqlite3_stmt {
    let mut st = std::ptr::null_mut();
    let sql = CString::new(sql).unwrap();
    assert_eq!(ffi::sqlite3_prepare_v2(c.handle(), sql.as_ptr(), -1, &mut st, std::ptr::null_mut()), ffi::SQLITE_OK);
    st
}

unsafe fn first(st: *mut ffi::sqlite3_stmt) -> Option<i64> {
    if ffi::sqlite3_step(st) == ffi::SQLITE_ROW { Some(ffi::sqlite3_column_int64(st, 0)) } else { None }
}

fn main() {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, a); INSERT INTO t(a) VALUES (1), (2);").unwrap();
    unsafe {
        let win = prep(&c, "SELECT id, count(a) OVER (ORDER BY id) FROM t");
        println!("{:?}", first(win).map(rusqlite::types::Value::Integer));
        let ins = prep(&c, "INSERT INTO t(a) VALUES (3) RETURNING id");
        println!("{:?}", first(ins).map(rusqlite::types::Value::Integer));
        let rc = ffi::sqlite3_reset(win);
        println!("{:?}", if rc == ffi::SQLITE_OK { Ok(()) } else { Err(rc) });
        let rc = ffi::sqlite3_step(ins);
        println!("{:?}", if rc == ffi::SQLITE_ROW || rc == ffi::SQLITE_DONE { Ok(rc == ffi::SQLITE_ROW) } else { Err(rc) });
        ffi::sqlite3_finalize(win);
        ffi::sqlite3_finalize(ins);
    }
    let n: i64 = c.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
    println!("count: {n}");
}
