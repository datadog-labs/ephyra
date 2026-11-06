// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! SQLite 3.50.4 side of issue_a_sibling.rs (rusqlite, WAL), printing the same lines.
use rusqlite::Connection;

fn main() {
    let a = Connection::open("t.db").unwrap();
    a.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(())).unwrap();
    let b = Connection::open("t.db").unwrap();
    a.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, x); INSERT INTO t(x) VALUES (1), (2), (3);").unwrap();

    let mut scan = b.prepare("SELECT id FROM t ORDER BY id").unwrap();
    let mut scan_rows = scan.raw_query();
    println!("scan: {:?}", scan_rows.next().unwrap().unwrap().get_ref(0).unwrap());

    // Another statement on B runs to completion while `scan` is open.
    let mut other = b.prepare("SELECT id FROM t WHERE id = 99").unwrap();
    println!("other: {:?}", other.raw_query().next().unwrap().is_some());
    drop(other);

    a.execute("INSERT INTO t(x) VALUES (4)", []).unwrap();

    println!("count on B: {}", b.query_row("SELECT count(*) FROM t", [], |r| Ok(format!("{:?}", r.get_ref(0).unwrap()))).unwrap());

    while let Some(r) = scan_rows.next().unwrap() {
        println!("scan: {:?}", r.get_ref(0).unwrap());
    }
    println!("scan done");
}
