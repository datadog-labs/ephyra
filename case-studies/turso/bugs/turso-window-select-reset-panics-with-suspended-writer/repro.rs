//! Resetting a window-function SELECT stepped partway, while an INSERT ...
//! RETURNING on the same connection is suspended after its first row,
//! panics in Turso (core/vdbe/mod.rs turso_assert).
use turso::Builder;

fn main() {
    let which = std::env::args().nth(1).unwrap_or_default();
    if which != "sqlite" {
        println!("== Turso");
        pollster::block_on(async {
            let db = Builder::new_local(":memory:").build().await.unwrap();
            let c = db.connect().unwrap();
            c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, a); INSERT INTO t(a) VALUES (1), (2);").await.unwrap();
            let mut ins = c.prepare("INSERT INTO t(a) VALUES (3) RETURNING id").await.unwrap();
            let mut win = c.prepare("SELECT id, count(a) OVER (ORDER BY id) FROM t").await.unwrap();
            let mut win_rows = win.query(()).await.unwrap();
            println!("window row: {:?}", win_rows.next().await.unwrap().map(|r| r.get_value(0).unwrap()));
            let mut ins_rows = ins.query(()).await.unwrap();
            println!("INSERT RETURNING row: {:?}", ins_rows.next().await.unwrap().map(|r| r.get_value(0).unwrap()));
            drop(win_rows);
            println!("resetting the window SELECT");
            println!("reset: {:?}", win.reset());
            println!("INSERT RETURNING next: {:?}", ins_rows.next().await.map(|r| r.is_some()));
        });
    }
    println!("== SQLite {}", rusqlite::version());
    let c = rusqlite::Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, a); INSERT INTO t(a) VALUES (1), (2);").unwrap();
    let mut ins = c.prepare("INSERT INTO t(a) VALUES (3) RETURNING id").unwrap();
    let mut win = c.prepare("SELECT id, count(a) OVER (ORDER BY id) FROM t").unwrap();
    let mut win_rows = win.raw_query();
    println!("window row: {:?}", win_rows.next().unwrap().map(|r| r.get::<_, i64>(0).unwrap()));
    let mut ins_rows = ins.raw_query();
    println!("INSERT RETURNING row: {:?}", ins_rows.next().unwrap().map(|r| r.get::<_, i64>(0).unwrap()));
    drop(win_rows); // sqlite3_reset
    println!("reset: ok");
    println!("INSERT RETURNING next: {:?}", ins_rows.next().map(|r| r.is_some()));
    drop(ins_rows);
    let n: i64 = c.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
    println!("count(*): {n}");
}
