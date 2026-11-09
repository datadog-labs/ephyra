//! SQLite 3.50.4 side of repro.rs (the code in posted.md) (rusqlite, WAL).
use rusqlite::{types::Value, Connection};

fn main() {
    let a = Connection::open("t.db").unwrap();
    a.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(())).unwrap();
    let b = Connection::open("t.db").unwrap();
    a.execute("ATTACH 'aux.db' AS aux", []).unwrap();
    b.execute("ATTACH 'aux.db' AS aux", []).unwrap();
    a.query_row("PRAGMA aux.journal_mode=WAL", [], |_| Ok(())).unwrap();
    a.execute_batch("CREATE TABLE aux.u(a); INSERT INTO aux.u VALUES (1);").unwrap();

    a.execute("BEGIN", []).unwrap();
    let n: i64 = a.query_row("SELECT count(*) FROM aux.u", [], |r| r.get(0)).unwrap();
    println!("{n}");
    b.execute("ALTER TABLE aux.u ADD COLUMN e", []).unwrap();
    let mut st = a.prepare("SELECT * FROM aux.u").unwrap();
    // column count read from the row, i.e. after any re-prepare at the step
    let row: Vec<Value> = st.query_row([], |r| (0..r.as_ref().column_count()).map(|i| r.get::<_, Value>(i)).collect()).unwrap();
    println!("SELECT * FROM aux.u: {row:?}");
    drop(st);
    let names: Vec<String> = a.prepare("SELECT name FROM aux.sqlite_schema").unwrap().query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
    println!("aux.sqlite_schema: {names:?}");
    a.execute("COMMIT", []).unwrap();
    let mut st = a.prepare("SELECT * FROM aux.u").unwrap();
    // column count read from the row, i.e. after any re-prepare at the step
    let row: Vec<Value> = st.query_row([], |r| (0..r.as_ref().column_count()).map(|i| r.get::<_, Value>(i)).collect()).unwrap();
    println!("after COMMIT: {row:?}");
}
