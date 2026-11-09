//! SQLite 3.50.4 side of repro.rs (the code in posted.md) (rusqlite, WAL, busy timeout 0), printing the same lines.
use rusqlite::Connection;

fn main() {
    let a = Connection::open("t.db").unwrap();
    a.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(())).unwrap();
    let b = Connection::open("t.db").unwrap();
    b.busy_timeout(std::time::Duration::ZERO).unwrap();
    a.execute_batch("CREATE TABLE t(x); CREATE INDEX i ON t(x);").unwrap();

    a.execute("BEGIN IMMEDIATE", []).unwrap();

    for sql in [
        "CREATE TABLE IF NOT EXISTS t(x)",
        "CREATE INDEX IF NOT EXISTS i ON t(x)",
        "DROP TABLE IF EXISTS nosuch",
        "DROP INDEX IF EXISTS nosuch",
    ] {
        println!("{sql}: {:?}", b.execute(sql, []));
    }
}
