// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! last_insert_rowid() after a SELECT with a window function.
use turso::Builder;

const SQL: &[&str] = &[
    "CREATE TABLE t(id INTEGER PRIMARY KEY, x)",
    "INSERT INTO t(x) VALUES (10), (20), (30), (40)",
    "INSERT INTO t(id, x) VALUES (100, 50)",
    "SELECT id, count(x) OVER (ORDER BY id) FROM t",
    "SELECT id, row_number() OVER () FROM t WHERE id < 3",
];

fn main() {
    println!("== Turso");
    pollster::block_on(async {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let c = db.connect().unwrap();
        for q in SQL {
            let mut r = c.query(q, ()).await.unwrap();
            while r.next().await.unwrap().is_some() {}
            drop(r);
            let mut x = c.query("SELECT last_insert_rowid()", ()).await.unwrap();
            println!("{q}; SELECT last_insert_rowid() -> {:?} (API: {})", x.next().await.unwrap().unwrap().get_value(0).unwrap(), c.last_insert_rowid());
        }
    });
    println!("== SQLite {}", rusqlite::version());
    let c = rusqlite::Connection::open_in_memory().unwrap();
    for q in SQL {
        let mut st = c.prepare(q).unwrap();
        let mut rows = st.raw_query();
        while rows.next().unwrap().is_some() {}
        drop(rows);
        let n: i64 = c.query_row("SELECT last_insert_rowid()", [], |r| r.get(0)).unwrap();
        println!("{q}; SELECT last_insert_rowid() -> {n} (API: {})", c.last_insert_rowid());
    }
}
