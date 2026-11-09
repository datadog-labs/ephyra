//! SQLite 3.50.4 side of repro.rs (the code in posted.md) (rusqlite, WAL), printing the same line.
use rusqlite::Connection;

fn main() {
    let a = Connection::open("t.db").unwrap();
    a.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(())).unwrap();
    let b = Connection::open("t.db").unwrap();
    a.execute_batch("CREATE TABLE t0(a); CREATE TABLE t1(a); INSERT INTO t1 VALUES (1);").unwrap();

    a.execute("BEGIN", []).unwrap();
    a.execute("SAVEPOINT s0", []).unwrap();
    let mut sel = a.prepare("SELECT a FROM t1").unwrap();

    b.execute("CREATE INDEX i ON t0(a)", []).unwrap();

    a.execute("ROLLBACK TO s0", []).unwrap();
    let mut rows = sel.raw_query();
    println!("{:?}", rows.next().map(|r| r.map(|r| r.get_ref(0).unwrap())));
}
