// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Variant of suspended_select_loses_rows: BEGIN; COMMIT on B (instead of a
//! sibling SELECT) ends the read transaction the suspended SELECT uses.
use turso::{Builder, Value};

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let path = path.to_str().unwrap();
    println!("== Turso");
    pollster::block_on(async {
        let db = Builder::new_local(path).build().await.unwrap();
        let a = db.connect().unwrap();
        let b = db.connect().unwrap();
        a.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, x); INSERT INTO t(x) VALUES (1), (2), (3);").await.unwrap();
        let mut scan = b.prepare("SELECT id, x FROM t ORDER BY id").await.unwrap();
        let mut scan_rows = scan.query(()).await.unwrap();
        let r = scan_rows.next().await.unwrap().unwrap();
        println!("scan row: {:?}", r.get_value(0).unwrap());
        b.execute("BEGIN", ()).await.unwrap();
        b.execute("COMMIT", ()).await.unwrap();
        println!("BEGIN; COMMIT on B");
        a.execute("INSERT INTO t(x) VALUES (4)", ()).await.unwrap();
        let mut c = b.query("SELECT count(*) FROM t", ()).await.unwrap();
        println!("count(*) on B: {:?}", c.next().await.unwrap().unwrap().get_value(0).unwrap());
        drop(c);
        loop {
            match scan_rows.next().await {
                Ok(Some(r)) => println!("scan row: {:?}", r.get_value(0).unwrap()),
                Ok(None) => {
                    println!("scan: done");
                    break;
                },
                Err(e) => {
                    println!("scan: error {e}");
                    break;
                },
            }
        }
        let _ = Value::Null;
    });
    drop(dir);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    println!("== SQLite {}", rusqlite::version());
    let a = rusqlite::Connection::open(&path).unwrap();
    a.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(id INTEGER PRIMARY KEY, x); INSERT INTO t(x) VALUES (1), (2), (3);").unwrap();
    let b = rusqlite::Connection::open(&path).unwrap();
    let mut scan = b.prepare("SELECT id, x FROM t ORDER BY id").unwrap();
    let mut scan_rows = scan.raw_query();
    println!("scan row: {:?}", scan_rows.next().unwrap().unwrap().get_ref(0).unwrap());
    b.execute_batch("BEGIN; COMMIT;").unwrap();
    println!("BEGIN; COMMIT on B");
    a.execute("INSERT INTO t(x) VALUES (4)", []).unwrap();
    let n: i64 = b.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
    println!("count(*) on B: {n}");
    loop {
        match scan_rows.next() {
            Ok(Some(r)) => println!("scan row: {:?}", r.get_ref(0).unwrap()),
            Ok(None) => {
                println!("scan: done");
                break;
            },
            Err(e) => {
                println!("scan: error {e}");
                break;
            },
        }
    }
}
