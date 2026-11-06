// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

// Variants: RELEASE instead of ROLLBACK TO; no SAVEPOINT; SELECT prepared after ROLLBACK TO.
use turso::Builder;

async fn run(label: &str, savepoint: bool, end: &str, prepare_late: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let db = Builder::new_local(path.to_str().unwrap()).build().await.unwrap();
    let a = db.connect().unwrap();
    let b = db.connect().unwrap();
    a.execute_batch("CREATE TABLE t0(a); CREATE TABLE t1(a); INSERT INTO t1 VALUES (1);").await.unwrap();
    a.execute("BEGIN", ()).await.unwrap();
    if savepoint { a.execute("SAVEPOINT s0", ()).await.unwrap(); }
    let mut sel = if prepare_late { None } else { Some(a.prepare("SELECT a FROM t1").await.unwrap()) };
    b.execute("CREATE INDEX i ON t0(a)", ()).await.unwrap();
    if !end.is_empty() { a.execute(end, ()).await.unwrap(); }
    if sel.is_none() { sel = Some(a.prepare("SELECT a FROM t1").await.unwrap()); }
    let mut sel = sel.unwrap();
    let mut rows = sel.query(()).await.unwrap();
    println!("{label}: {:?}", rows.next().await.map(|r| r.map(|r| r.get_value(0).unwrap())));
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    run("ROLLBACK TO", true, "ROLLBACK TO s0", false).await;
    run("RELEASE", true, "RELEASE s0", false).await;
    run("no savepoint", false, "", false).await;
    run("prepared after ROLLBACK TO", true, "ROLLBACK TO s0", true).await;
}
