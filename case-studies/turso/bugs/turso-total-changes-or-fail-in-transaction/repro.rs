//! Inside BEGIN, an INSERT OR FAIL that inserts rows and then fails on a
//! constraint: changes() counts the inserted rows, total_changes() does not.
use turso::Builder;

const SQL: &[&str] = &[
    "CREATE TABLE t(a UNIQUE)",
    "INSERT INTO t VALUES (1)",
    "BEGIN",
    "INSERT OR FAIL INTO t VALUES (2), (3), (1)",
];

fn main() {
    println!("== Turso");
    pollster::block_on(async {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let c = db.connect().unwrap();
        for q in SQL {
            let r = c.execute(q, ()).await;
            let mut x = c.query("SELECT changes(), total_changes()", ()).await.unwrap();
            let row = x.next().await.unwrap().unwrap();
            println!("{q}: {:?}; changes(), total_changes() = {:?}, {:?}", r.map(|_| ()), row.get_value(0).unwrap(), row.get_value(1).unwrap());
        }
    });
    println!("== SQLite {}", rusqlite::version());
    let c = rusqlite::Connection::open_in_memory().unwrap();
    for q in SQL {
        let r = c.execute(q, []);
        let (a, b): (i64, i64) = c.query_row("SELECT changes(), total_changes()", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        println!("{q}: {:?}; changes(), total_changes() = {a}, {b}", r.map(|_| ()));
    }
}
