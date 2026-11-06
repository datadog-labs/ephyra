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
    drop(rows);
    a.execute("INSERT INTO t VALUES (1)", ()).await.unwrap();

    println!("{:?}", b.execute("INSERT INTO aux.u VALUES (1)", ()).await);
    println!("{:?}", b.execute("ALTER TABLE aux.u ADD COLUMN e", ()).await);
    println!("{:?}", b.execute("CREATE INDEX aux.i ON u(a)", ()).await);
    b.execute("ROLLBACK", ()).await.unwrap();

    // 2. While b's transaction holds DDL on aux, a cannot write to main.
    b.execute("BEGIN", ()).await.unwrap();
    b.execute("ALTER TABLE aux.u ADD COLUMN e", ()).await.unwrap();
    println!("{:?}", a.execute("INSERT INTO t VALUES (2)", ()).await);
    b.execute("COMMIT", ()).await.unwrap();
}
