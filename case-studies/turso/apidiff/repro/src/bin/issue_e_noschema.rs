// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

// Control: same steps without the schema change.
use turso::Builder;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db = Builder::new_local("t.db").build().await.unwrap();
    let c = db.connect().unwrap();
    c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, x); CREATE TABLE u(y);").await.unwrap();
    let mut sel = c.prepare("SELECT y FROM u").await.unwrap();
    let mut ins = c.prepare("INSERT INTO t(x) VALUES (1) RETURNING id").await.unwrap();
    let mut ins_rows = ins.query(()).await.unwrap();
    println!("{:?}", ins_rows.next().await.unwrap().map(|r| r.get_value(0).unwrap()));
    let mut sel_rows = sel.query(()).await.unwrap();
    println!("{:?}", sel_rows.next().await.map(|r| r.is_some()));
    drop(sel_rows);
    println!("{:?}", ins_rows.next().await.map(|r| r.is_some()));
    drop(ins_rows);
    let c2 = db.connect().unwrap();
    let mut n = c2.query("SELECT count(*) FROM t", ()).await.unwrap();
    println!("{:?}", n.next().await.unwrap().unwrap().get_value(0).unwrap());
}
