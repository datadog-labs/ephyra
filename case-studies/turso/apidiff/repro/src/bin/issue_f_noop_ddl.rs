// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

use turso::Builder;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db = Builder::new_local("t.db").build().await.unwrap();
    let a = db.connect().unwrap();
    let b = db.connect().unwrap();
    a.execute_batch("CREATE TABLE t(x); CREATE INDEX i ON t(x);").await.unwrap();

    a.execute("BEGIN IMMEDIATE", ()).await.unwrap();

    for sql in [
        "CREATE TABLE IF NOT EXISTS t(x)",
        "CREATE INDEX IF NOT EXISTS i ON t(x)",
        "DROP TABLE IF EXISTS nosuch",
        "DROP INDEX IF EXISTS nosuch",
    ] {
        println!("{sql}: {:?}", b.execute(sql, ()).await);
    }
}
