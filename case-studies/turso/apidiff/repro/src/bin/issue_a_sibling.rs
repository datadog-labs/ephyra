// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

use turso::Builder;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db = Builder::new_local("t.db").build().await.unwrap();
    let a = db.connect().unwrap();
    let b = db.connect().unwrap();
    a.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, x); INSERT INTO t(x) VALUES (1), (2), (3);").await.unwrap();

    let mut scan = b.prepare("SELECT id FROM t ORDER BY id").await.unwrap();
    let mut scan_rows = scan.query(()).await.unwrap();
    println!("scan: {:?}", scan_rows.next().await.unwrap().unwrap().get_value(0).unwrap());

    // Another statement on B runs to completion while `scan` is open.
    let mut other = b.query("SELECT id FROM t WHERE id = 99", ()).await.unwrap();
    println!("other: {:?}", other.next().await.unwrap().is_some());
    drop(other);

    a.execute("INSERT INTO t(x) VALUES (4)", ()).await.unwrap();

    let mut n = b.query("SELECT count(*) FROM t", ()).await.unwrap();
    println!("count on B: {:?}", n.next().await.unwrap().unwrap().get_value(0).unwrap());
    drop(n);

    while let Some(r) = scan_rows.next().await.unwrap() {
        println!("scan: {:?}", r.get_value(0).unwrap());
    }
    println!("scan done");
}
