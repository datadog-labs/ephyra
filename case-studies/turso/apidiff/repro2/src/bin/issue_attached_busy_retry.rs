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
    a.execute("CREATE TABLE aux.u(x)", ()).await.unwrap();

    a.execute("BEGIN", ()).await.unwrap();
    a.execute("INSERT INTO aux.u VALUES (1)", ()).await.unwrap();

    let mut ins = b.prepare("INSERT INTO aux.u VALUES (2)").await.unwrap();
    let mut rows = ins.query(()).await.unwrap();
    println!("{:?}", rows.next().await);
    // Turso: Err(Busy("database is locked"))
    // SQLite 3.50.4: SQLITE_BUSY

    a.execute("COMMIT", ()).await.unwrap();

    println!("{:?}", rows.next().await);
    // Turso: Err(Busy("database is locked"))
    // SQLite 3.50.4: SQLITE_DONE
    println!("{:?}", rows.next().await);
    // Turso: Err(Busy("database is locked"))

    drop(rows);
    ins.reset().unwrap();
    println!("{:?}", ins.execute(()).await);
    // Turso: Ok(1)
}
