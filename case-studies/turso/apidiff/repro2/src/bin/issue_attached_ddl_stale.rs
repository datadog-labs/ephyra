// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

use turso::Builder;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db = Builder::new_local("t.db").experimental_attach(true).build().await.unwrap();
    let a = db.connect().unwrap();
    let b = db.connect().unwrap();
    a.execute("ATTACH 'aux.db' AS aux", ()).await.unwrap();
    b.execute("ATTACH 'aux.db' AS aux", ()).await.unwrap();
    a.execute_batch("CREATE TABLE t(x); CREATE TABLE aux.u(a);").await.unwrap();

    // 1. b's transaction has read main; a then commits a change to main only.
    b.execute("BEGIN", ()).await.unwrap();
    let mut rows = b.query("SELECT count(*) FROM t", ()).await.unwrap();
    println!("{:?}", rows.next().await.unwrap().unwrap().get_value(0).unwrap());
    // Turso: Integer(0)
    // SQLite 3.50.4: 0
    drop(rows);
    a.execute("INSERT INTO t VALUES (1)", ()).await.unwrap();

    println!("{:?}", b.execute("INSERT INTO aux.u VALUES (1)", ()).await);
    // Turso: Ok(1)
    // SQLite 3.50.4: Ok(1)
    println!("{:?}", b.execute("ALTER TABLE aux.u ADD COLUMN e", ()).await);
    // Turso: Err(BusySnapshot("database snapshot is stale, rollback and retry the transaction"))
    // SQLite 3.50.4: Ok(0)
    println!("{:?}", b.execute("CREATE INDEX aux.i ON u(a)", ()).await);
    // Turso: Err(BusySnapshot("database snapshot is stale, rollback and retry the transaction"))
    // SQLite 3.50.4: Ok(0)
    b.execute("ROLLBACK", ()).await.unwrap();

    // 2. While b's transaction holds DDL on aux, a cannot write to main.
    b.execute("BEGIN", ()).await.unwrap();
    b.execute("ALTER TABLE aux.u ADD COLUMN e", ()).await.unwrap();
    println!("{:?}", a.execute("INSERT INTO t VALUES (2)", ()).await);
    // Turso: Err(Busy("database is locked"))
    // SQLite 3.50.4: Ok(1)
    b.execute("COMMIT", ()).await.unwrap();
}
