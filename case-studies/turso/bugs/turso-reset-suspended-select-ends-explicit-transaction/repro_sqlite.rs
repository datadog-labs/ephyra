//! SQLite 3.50.4 side of repro.rs (the code in posted.md) (rusqlite), printing the same lines.
//! Dropping rusqlite's Rows resets the statement (sqlite3_reset).
use rusqlite::Connection;

fn main() {
    let c = Connection::open("t.db").unwrap();
    c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1), (2);").unwrap();

    let mut s = c.prepare("SELECT x FROM t").unwrap();
    let mut rows = s.raw_query();
    println!("{:?}", rows.next().unwrap().unwrap().get_ref(0).unwrap());

    c.execute("BEGIN", []).unwrap();
    c.execute("INSERT INTO t VALUES (3)", []).unwrap();
    println!("autocommit: {}", c.is_autocommit());

    drop(rows);
    println!("autocommit: {}", c.is_autocommit());

    println!("{}", c.query_row("SELECT count(*) FROM t", [], |r| Ok(format!("{:?}", r.get_ref(0).unwrap()))).unwrap());

    println!("{:?}", c.execute("COMMIT", []));
}
