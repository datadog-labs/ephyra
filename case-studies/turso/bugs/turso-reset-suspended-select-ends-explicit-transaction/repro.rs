//! Resetting (or dropping) a SELECT that was stepped partway, after BEGIN
//! and an INSERT ran on the same connection, ends the transaction in Turso
//! and discards the INSERT.
use turso::Builder;

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    println!("== Turso");
    pollster::block_on(async {
        let db = Builder::new_local(path.to_str().unwrap()).build().await.unwrap();
        let c = db.connect().unwrap();
        c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1), (2);").await.unwrap();
        let mut s = c.prepare("SELECT x FROM t").await.unwrap();
        let mut rows = s.query(()).await.unwrap();
        println!("row: {:?}", rows.next().await.unwrap().unwrap().get_value(0).unwrap());
        c.execute("BEGIN", ()).await.unwrap();
        c.execute("INSERT INTO t VALUES (3)", ()).await.unwrap();
        println!("autocommit before reset: {}", c.is_autocommit().unwrap());
        drop(rows);
        s.reset().unwrap();
        println!("autocommit after reset: {}", c.is_autocommit().unwrap());
        let mut n = c.query("SELECT count(*) FROM t", ()).await.unwrap();
        println!("count(*): {:?}", n.next().await.unwrap().unwrap().get_value(0).unwrap());
        drop(n);
        println!("COMMIT: {:?}", c.execute("COMMIT", ()).await);
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    println!("== SQLite {}", rusqlite::version());
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1), (2);").unwrap();
    let mut s = c.prepare("SELECT x FROM t").unwrap();
    {
        let mut rows = s.raw_query();
        println!("row: {:?}", rows.next().unwrap().unwrap().get_ref(0).unwrap());
        c.execute("BEGIN", []).unwrap();
        c.execute("INSERT INTO t VALUES (3)", []).unwrap();
        println!("autocommit before reset: {}", c.is_autocommit());
    } // dropping rusqlite's Rows resets the statement (sqlite3_reset)
    println!("autocommit after reset: {}", c.is_autocommit());
    let n: i64 = c.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
    println!("count(*): {n}");
    println!("COMMIT: {:?}", c.execute("COMMIT", []));
}
