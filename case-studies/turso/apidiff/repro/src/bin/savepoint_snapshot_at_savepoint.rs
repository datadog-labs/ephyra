// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! SAVEPOINT outside a transaction takes the read snapshot at once in
//! Turso; SQLite starts a deferred transaction (as BEGIN does), so a
//! write after another connection's commit succeeds there.
use turso::Builder;

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    println!("== Turso");
    pollster::block_on(async {
        let db = Builder::new_local(path.to_str().unwrap()).build().await.unwrap();
        let a = db.connect().unwrap();
        let b = db.connect().unwrap();
        a.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1);").await.unwrap();
        for begin in ["BEGIN", "SAVEPOINT sp"] {
            b.execute(begin, ()).await.unwrap();
            a.execute("INSERT INTO t VALUES (2)", ()).await.unwrap();
            println!("{begin}; (other connection commits); INSERT: {:?}", b.execute("INSERT INTO t VALUES (3)", ()).await);
            let _ = b.execute("ROLLBACK", ()).await;
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    println!("== SQLite {}", rusqlite::version());
    let a = rusqlite::Connection::open(&path).unwrap();
    a.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(x); INSERT INTO t VALUES (1);").unwrap();
    let b = rusqlite::Connection::open(&path).unwrap();
    for begin in ["BEGIN", "SAVEPOINT sp"] {
        b.execute(begin, []).unwrap();
        a.execute("INSERT INTO t VALUES (2)", []).unwrap();
        println!("{begin}; (other connection commits); INSERT: {:?}", b.execute("INSERT INTO t VALUES (3)", []));
        let _ = b.execute("ROLLBACK", []);
    }
}
