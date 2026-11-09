# A completed INSERT ... RETURNING is rolled back when a statement prepared before a schema change is stepped while the INSERT is suspended

Posted as https://github.com/tursodatabase/turso/issues/11540 on 2026-11-06. The text below is the issue as posted.

---

### Reproduction

Turso main 52d6fd81d, with the `turso` crate (tokio), file database, default options.

```rust
use turso::Builder;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db = Builder::new_local("t.db").build().await.unwrap();
    let c = db.connect().unwrap();
    c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, x); CREATE TABLE u(y);").await.unwrap();

    let mut sel = c.prepare("SELECT y FROM u").await.unwrap();
    c.execute("CREATE TABLE v(z)", ()).await.unwrap(); // schema change: `sel` is now stale

    let mut ins = c.prepare("INSERT INTO t(x) VALUES (1) RETURNING id").await.unwrap();
    let mut ins_rows = ins.query(()).await.unwrap();
    println!("{:?}", ins_rows.next().await.unwrap().map(|r| r.get_value(0).unwrap()));

    let mut sel_rows = sel.query(()).await.unwrap();
    println!("{:?}", sel_rows.next().await.map(|r| r.is_some()));
    drop(sel_rows);

    println!("{:?}", ins_rows.next().await.map(|r| r.is_some()));
    drop(ins_rows);

    let mut n = c.query("SELECT count(*) FROM t", ()).await.unwrap();
    println!("{:?}", n.next().await.unwrap().unwrap().get_value(0).unwrap());
    drop(n);

    let c2 = db.connect().unwrap();
    let mut n = c2.query("SELECT count(*) FROM t", ()).await.unwrap();
    println!("{:?}", n.next().await.unwrap().unwrap().get_value(0).unwrap());
}
```

### Current behavior

The INSERT finishes without an error, but its row is gone, on this connection and on a new one:

```
Some(Integer(1))
Ok(false)
Ok(false)
Integer(0)
Integer(0)
```

### Expected behavior

The row is kept. SQLite 3.50.4 (the same steps through rusqlite) prints:

```
Some(Integer(1))
Ok(false)
Ok(false)
Integer(1)
Integer(1)
```

The row is also lost when the INSERT is reset after its first row instead of being stepped to the end; without the `CREATE TABLE v(z)` step Turso keeps it. The rollback comes from `Statement::finish_step` calling `rollback_autocommit_txn()` before re-preparing the stale SELECT (`core/statement.rs:627-629`), which rolls back the suspended INSERT's write transaction (`core/statement.rs:956-963`). Same symptom as #7260 / #7345 (fixed), through a different path.
