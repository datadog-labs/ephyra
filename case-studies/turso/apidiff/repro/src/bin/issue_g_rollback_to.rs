// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

use turso::Builder;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db = Builder::new_local("t.db").build().await.unwrap();
    let a = db.connect().unwrap();
    let b = db.connect().unwrap();
    a.execute_batch("CREATE TABLE t0(a); CREATE TABLE t1(a); INSERT INTO t1 VALUES (1);").await.unwrap();

    a.execute("BEGIN", ()).await.unwrap();
    a.execute("SAVEPOINT s0", ()).await.unwrap();
    let mut sel = a.prepare("SELECT a FROM t1").await.unwrap();

    b.execute("CREATE INDEX i ON t0(a)", ()).await.unwrap();

    a.execute("ROLLBACK TO s0", ()).await.unwrap();
    let mut rows = sel.query(()).await.unwrap();
    println!("{:?}", rows.next().await.map(|r| r.map(|r| r.get_value(0).unwrap())));
}
