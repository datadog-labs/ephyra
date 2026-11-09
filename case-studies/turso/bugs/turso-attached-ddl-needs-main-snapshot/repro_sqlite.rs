//! SQLite 3.50.4 side of repro.rs (the code in posted.md) (rusqlite, WAL).
use rusqlite::Connection;

fn main() {
    let a = Connection::open("t.db").unwrap();
    a.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(())).unwrap();
    let b = Connection::open("t.db").unwrap();
    a.busy_timeout(std::time::Duration::ZERO).unwrap();
    b.busy_timeout(std::time::Duration::ZERO).unwrap();
    a.execute("ATTACH 'aux.db' AS aux", []).unwrap();
    b.execute("ATTACH 'aux.db' AS aux", []).unwrap();
    a.query_row("PRAGMA aux.journal_mode=WAL", [], |_| Ok(())).unwrap();
    a.execute_batch("CREATE TABLE t(x); CREATE TABLE aux.u(a);").unwrap();

    b.execute("BEGIN", []).unwrap();
    let n: i64 = b.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
    println!("{n}");
    a.execute("INSERT INTO t VALUES (1)", []).unwrap();
    println!("{:?}", b.execute("INSERT INTO aux.u VALUES (1)", []));
    println!("{:?}", b.execute("ALTER TABLE aux.u ADD COLUMN e", []));
    println!("{:?}", b.execute("CREATE INDEX aux.i ON u(a)", []));
    b.execute("ROLLBACK", []).unwrap();

    b.execute("BEGIN", []).unwrap();
    b.execute("ALTER TABLE aux.u ADD COLUMN e", []).unwrap();
    println!("{:?}", a.execute("INSERT INTO t VALUES (2)", []));
    b.execute("COMMIT", []).unwrap();
}
