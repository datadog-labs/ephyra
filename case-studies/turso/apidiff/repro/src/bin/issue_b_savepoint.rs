// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

// Variant: SAVEPOINT instead of BEGIN, and drop instead of reset.
use turso::Builder;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db = Builder::new_local("t.db").build().await.unwrap();
    let c = db.connect().unwrap();
    c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1), (2);").await.unwrap();
    let mut s = c.prepare("SELECT x FROM t").await.unwrap();
    let mut rows = s.query(()).await.unwrap();
    println!("{:?}", rows.next().await.unwrap().unwrap().get_value(0).unwrap());
    c.execute("SAVEPOINT sp", ()).await.unwrap();
    c.execute("INSERT INTO t VALUES (3)", ()).await.unwrap();
    drop(rows);
    drop(s);
    println!("autocommit: {}", c.is_autocommit().unwrap());
    println!("ROLLBACK TO: {:?}", c.execute("ROLLBACK TO sp", ()).await);
    println!("RELEASE: {:?}", c.execute("RELEASE sp", ()).await);
    let mut n = c.query("SELECT count(*) FROM t", ()).await.unwrap();
    println!("{:?}", n.next().await.unwrap().unwrap().get_value(0).unwrap());
}
