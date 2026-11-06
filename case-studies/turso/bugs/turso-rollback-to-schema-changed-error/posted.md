# "Database schema changed" from a statement stepped after ROLLBACK TO, when another connection changed the schema

Posted as https://github.com/tursodatabase/turso/issues/11537 on 2026-11-06. The text below is the issue as posted.

---

### Reproduction

Turso main 52d6fd81d, with the `turso` crate (tokio), file database.

```rust
use turso::Builder;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db = Builder::new_local("t.db").build().await.unwrap();
    let a = db.connect().unwrap();
    let b = db.connect().unwrap();
    a.execute_batch("CREATE TABLE t0(a); CREATE TABLE t1(a); INSERT INTO t1 VALUES (1);").await.unwrap();

    a.execute("BEGIN", ()).await.unwrap();
    a.execute("SAVEPOINT s0", ()).await.unwrap();
    let mut sel = a.prepare("SELECT a FROM t1").await.unwrap();

    b.execute("CREATE INDEX i ON t0(a)", ()).await.unwrap();

    a.execute("ROLLBACK TO s0", ()).await.unwrap();
    let mut rows = sel.query(()).await.unwrap();
    println!("{:?}", rows.next().await.map(|r| r.map(|r| r.get_value(0).unwrap())));
}
```

### Current behavior

The SELECT fails although nothing it reads changed:

```
Err(Error("Database schema changed"))
```

### Expected behavior

The SELECT returns the row. SQLite 3.50.4 (the same steps through rusqlite, journal_mode=WAL) prints:

```
Ok(Some(Integer(1)))
```

Turso returns the row with `RELEASE s0` instead of `ROLLBACK TO s0`, without the SAVEPOINT, or when the SELECT is prepared after the `ROLLBACK TO`. The re-prepare after `ROLLBACK TO` takes the newer shared schema (`refresh_schema_from_shared_for_reprepare`, `core/connection.rs:2193-2203`) while A's snapshot still has the old cookie, so every retry ends in `SchemaUpdated` (`core/vdbe/execute.rs:5060-5066`). Possibly related: #5930 (closed), #10975 and #11035 (open).
