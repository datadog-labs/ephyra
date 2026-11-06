use turso::Builder;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db = Builder::new_local(":memory:").build().await.unwrap();
    let c = db.connect().unwrap();
    c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, a); INSERT INTO t(a) VALUES (1), (2);").await.unwrap();

    let mut win = c.prepare("SELECT id, count(a) OVER (ORDER BY id) FROM t").await.unwrap();
    let mut win_rows = win.query(()).await.unwrap();
    println!("{:?}", win_rows.next().await.unwrap().map(|r| r.get_value(0).unwrap()));

    let mut ins = c.prepare("INSERT INTO t(a) VALUES (3) RETURNING id").await.unwrap();
    let mut ins_rows = ins.query(()).await.unwrap();
    println!("{:?}", ins_rows.next().await.unwrap().map(|r| r.get_value(0).unwrap()));

    drop(win_rows);
    println!("{:?}", win.reset());

    println!("{:?}", ins_rows.next().await.map(|r| r.is_some()));
}
