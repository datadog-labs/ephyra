// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! An INSERT ... RETURNING that ran to completion is lost when, while it
//! was suspended after its first row, another statement on the same
//! connection was stepped that had been prepared before a schema change.
use turso::Builder;

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    println!("== Turso");
    pollster::block_on(async {
        let db = Builder::new_local(path.to_str().unwrap()).build().await.unwrap();
        let c = db.connect().unwrap();
        c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, x); CREATE TABLE u(y);").await.unwrap();
        let mut sel = c.prepare("SELECT y FROM u").await.unwrap();
        c.execute("CREATE TABLE v(z)", ()).await.unwrap(); // schema change: `sel` must be re-prepared
        let mut ins = c.prepare("INSERT INTO t(x) VALUES (1) RETURNING id").await.unwrap();
        let mut ins_rows = ins.query(()).await.unwrap();
        println!("INSERT RETURNING row: {:?}", ins_rows.next().await.unwrap().map(|r| r.get_value(0).unwrap()));
        let mut sel_rows = sel.query(()).await.unwrap();
        println!("SELECT: {:?}", sel_rows.next().await.map(|r| r.is_some()));
        drop(sel_rows);
        println!("INSERT RETURNING next: {:?}", ins_rows.next().await.map(|r| r.is_some()));
        drop(ins_rows);
        let mut n = c.query("SELECT count(*) FROM t", ()).await.unwrap();
        println!("count(*) FROM t: {:?}", n.next().await.unwrap().unwrap().get_value(0).unwrap());
        drop(n);
        let c2 = db.connect().unwrap();
        let mut n = c2.query("SELECT count(*) FROM t", ()).await.unwrap();
        println!("count(*) FROM t on a new connection: {:?}", n.next().await.unwrap().unwrap().get_value(0).unwrap());
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    println!("== SQLite {}", rusqlite::version());
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, x); CREATE TABLE u(y);").unwrap();
    let mut sel = c.prepare("SELECT y FROM u").unwrap();
    c.execute("CREATE TABLE v(z)", []).unwrap();
    let mut ins = c.prepare("INSERT INTO t(x) VALUES (1) RETURNING id").unwrap();
    {
        let mut ins_rows = ins.raw_query();
        println!("INSERT RETURNING row: {:?}", ins_rows.next().unwrap().map(|r| r.get::<_, i64>(0).unwrap()));
        {
            let mut sel_rows = sel.raw_query();
            println!("SELECT: {:?}", sel_rows.next().map(|r| r.is_some()));
        }
        println!("INSERT RETURNING next: {:?}", ins_rows.next().map(|r| r.is_some()));
    }
    let n: i64 = c.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
    println!("count(*) FROM t: {n}");
    let c2 = rusqlite::Connection::open(&path).unwrap();
    let n: i64 = c2.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
    println!("count(*) FROM t on a new connection: {n}");
}
