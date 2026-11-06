//! CREATE TABLE IF NOT EXISTS on an existing table and DROP INDEX IF EXISTS
//! on a missing index fail with "database is locked" while another
//! connection holds a write transaction. SQLite runs them without a write
//! lock and succeeds.
use turso::Builder;

const SQL: &[&str] = &["CREATE TABLE IF NOT EXISTS t(x)", "DROP INDEX IF EXISTS nosuch", "DROP TABLE IF EXISTS nosuch", "CREATE INDEX IF NOT EXISTS i ON t(x)"];

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    println!("== Turso");
    pollster::block_on(async {
        let db = Builder::new_local(path.to_str().unwrap()).build().await.unwrap();
        let a = db.connect().unwrap();
        let b = db.connect().unwrap();
        a.execute_batch("CREATE TABLE t(x); CREATE INDEX i ON t(x);").await.unwrap();
        a.execute("BEGIN IMMEDIATE", ()).await.unwrap();
        for q in SQL {
            println!("{q}: {:?}", b.execute(q, ()).await);
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    println!("== SQLite {}", rusqlite::version());
    let a = rusqlite::Connection::open(&path).unwrap();
    a.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(x); CREATE INDEX i ON t(x);").unwrap();
    let b = rusqlite::Connection::open(&path).unwrap();
    a.execute("BEGIN IMMEDIATE", []).unwrap();
    for q in SQL {
        println!("{q}: {:?}", b.execute(q, []));
    }
}
