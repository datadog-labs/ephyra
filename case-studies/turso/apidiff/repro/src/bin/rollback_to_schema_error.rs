// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A statement prepared inside BEGIN/SAVEPOINT, stepped after another
//! connection's CREATE INDEX and a ROLLBACK TO, fails with "Database schema
//! changed" in Turso; SQLite re-prepares it.
use turso::Builder;

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    println!("== Turso");
    pollster::block_on(async {
        let db = Builder::new_local(path.to_str().unwrap()).build().await.unwrap();
        let a = db.connect().unwrap();
        let b = db.connect().unwrap();
        a.execute_batch("CREATE TABLE t0(a); CREATE TABLE t1(a); INSERT INTO t1 VALUES (1);").await.unwrap();
        a.execute("BEGIN", ()).await.unwrap();
        a.execute("SAVEPOINT s0", ()).await.unwrap();
        let mut sel = a.prepare("SELECT a FROM t1").await.unwrap();
        b.execute("CREATE INDEX i ON t0(a)", ()).await.unwrap();
        a.execute("ROLLBACK TO s0", ()).await.unwrap();
        let mut rows = sel.query(()).await.unwrap();
        println!("step: {:?}", rows.next().await.map(|r| r.map(|r| r.get_value(0).unwrap())));
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    println!("== SQLite {}", rusqlite::version());
    let a = rusqlite::Connection::open(&path).unwrap();
    a.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t0(a); CREATE TABLE t1(a); INSERT INTO t1 VALUES (1);").unwrap();
    let b = rusqlite::Connection::open(&path).unwrap();
    a.execute("BEGIN", []).unwrap();
    a.execute("SAVEPOINT s0", []).unwrap();
    let mut sel = a.prepare("SELECT a FROM t1").unwrap();
    b.execute("CREATE INDEX i ON t0(a)", []).unwrap();
    a.execute("ROLLBACK TO s0", []).unwrap();
    let mut rows = sel.raw_query();
    println!("step: {:?}", rows.next().map(|r| r.map(|r| r.get::<_, i64>(0).unwrap())));
}
