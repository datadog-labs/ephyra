use turso::Builder;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db = Builder::new_local("t.db").experimental_attach(true).build().await.unwrap();
    let a = db.connect().unwrap();
    let b = db.connect().unwrap();
    a.execute("ATTACH 'aux.db' AS aux", ()).await.unwrap();
    b.execute("ATTACH 'aux.db' AS aux", ()).await.unwrap();
    a.execute_batch("CREATE TABLE aux.u(a); INSERT INTO aux.u VALUES (1);").await.unwrap();

    a.execute("BEGIN", ()).await.unwrap();
    let mut rows = a.query("SELECT count(*) FROM aux.u", ()).await.unwrap();
    println!("{:?}", rows.next().await.unwrap().unwrap().get_value(0).unwrap());
    drop(rows);

    b.execute("ALTER TABLE aux.u ADD COLUMN e", ()).await.unwrap();

    let mut rows = a.query("SELECT * FROM aux.u", ()).await.unwrap();
    println!("{:?}", rows.next().await.map(|r| r.map(|r| r.get_value(0).unwrap())));
    drop(rows);

    a.execute("COMMIT", ()).await.unwrap();
    let mut rows = a.query("SELECT * FROM aux.u", ()).await.unwrap();
    println!("{:?}", rows.next().await.map(|r| r.map(|r| (r.get_value(0).unwrap(), r.get_value(1).unwrap()))));
}
