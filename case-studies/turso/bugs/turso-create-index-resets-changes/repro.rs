//! changes() after CREATE INDEX.
use turso::Builder;

const SQL: &[&str] = &["CREATE TABLE t(x)", "INSERT INTO t VALUES (1), (2), (3)", "CREATE INDEX i ON t(x)"];

fn main() {
    println!("== Turso");
    pollster::block_on(async {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let c = db.connect().unwrap();
        for q in SQL {
            c.execute(q, ()).await.unwrap();
            let mut r = c.query("SELECT changes()", ()).await.unwrap();
            println!("{q}; SELECT changes() -> {:?}", r.next().await.unwrap().unwrap().get_value(0).unwrap());
        }
    });
    println!("== SQLite {}", rusqlite::version());
    let c = rusqlite::Connection::open_in_memory().unwrap();
    for q in SQL {
        c.execute(q, []).unwrap();
        let n: i64 = c.query_row("SELECT changes()", [], |r| r.get(0)).unwrap();
        println!("{q}; SELECT changes() -> {n}");
    }
}
