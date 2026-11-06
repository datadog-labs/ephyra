// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! SQLite 3.50.4 side of issue_attached_busy_retry.rs (rusqlite, WAL, raw
//! sqlite3_step so the statement is stepped again without a reset).
use rusqlite::{ffi, Connection};
use std::ffi::CString;

fn step_name(rc: i32) -> &'static str {
    match rc {
        ffi::SQLITE_BUSY => "SQLITE_BUSY",
        ffi::SQLITE_DONE => "SQLITE_DONE",
        ffi::SQLITE_ROW => "SQLITE_ROW",
        _ => "other",
    }
}

fn main() {
    let a = Connection::open("t.db").unwrap();
    a.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(())).unwrap();
    let b = Connection::open("t.db").unwrap();
    b.busy_timeout(std::time::Duration::ZERO).unwrap();
    a.execute("ATTACH 'aux.db' AS aux", []).unwrap();
    b.execute("ATTACH 'aux.db' AS aux", []).unwrap();
    a.query_row("PRAGMA aux.journal_mode=WAL", [], |_| Ok(())).unwrap();
    a.execute("CREATE TABLE aux.u(x)", []).unwrap();

    a.execute("BEGIN", []).unwrap();
    a.execute("INSERT INTO aux.u VALUES (1)", []).unwrap();

    unsafe {
        let mut st = std::ptr::null_mut();
        let sql = CString::new("INSERT INTO aux.u VALUES (2)").unwrap();
        assert_eq!(ffi::sqlite3_prepare_v2(b.handle(), sql.as_ptr(), -1, &mut st, std::ptr::null_mut()), ffi::SQLITE_OK);
        println!("{}", step_name(ffi::sqlite3_step(st)));
        a.execute("COMMIT", []).unwrap();
        println!("{}", step_name(ffi::sqlite3_step(st)));
        ffi::sqlite3_finalize(st);
    }
    let n: i64 = b.query_row("SELECT count(*) FROM aux.u", [], |r| r.get(0)).unwrap();
    println!("count: {n}");
}
