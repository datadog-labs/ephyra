// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! SQLite 3.50.4 side of issue_e_reprepare.rs (rusqlite), printing the same lines.
use rusqlite::Connection;

fn main() {
    let c = Connection::open("t.db").unwrap();
    c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, x); CREATE TABLE u(y);").unwrap();

    let mut sel = c.prepare("SELECT y FROM u").unwrap();
    c.execute("CREATE TABLE v(z)", []).unwrap(); // schema change: `sel` is now stale

    let mut ins = c.prepare("INSERT INTO t(x) VALUES (1) RETURNING id").unwrap();
    let mut ins_rows = ins.raw_query();
    println!("{:?}", ins_rows.next().unwrap().map(|r| r.get_ref(0).unwrap()));

    let mut sel_rows = sel.raw_query();
    println!("{:?}", sel_rows.next().map(|r| r.is_some()));
    drop(sel_rows);

    println!("{:?}", ins_rows.next().map(|r| r.is_some()));
    drop(ins_rows);

    println!("{}", c.query_row("SELECT count(*) FROM t", [], |r| Ok(format!("{:?}", r.get_ref(0).unwrap()))).unwrap());

    let c2 = Connection::open("t.db").unwrap();
    println!("{}", c2.query_row("SELECT count(*) FROM t", [], |r| Ok(format!("{:?}", r.get_ref(0).unwrap()))).unwrap());
}
