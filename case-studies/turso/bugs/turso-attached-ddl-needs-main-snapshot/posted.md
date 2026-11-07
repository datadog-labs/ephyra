# DDL on an attached database takes a write transaction on main: "database snapshot is stale" after another connection wrote to main, and writers of main get Busy

Posted as https://github.com/tursodatabase/turso/issues/11539 on 2026-11-06. The text below is the issue as posted.

---

### Reproduction

Turso main 52d6fd81d, with the `turso` crate (tokio), file databases, `experimental_attach(true)`.

```rust
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
```

### Current behavior

B's INSERT into `aux.u` works, but its ALTER TABLE and CREATE INDEX on `aux` fail because main moved on; and while B holds DDL on `aux`, A cannot write to main:

```
Integer(0)
Ok(1)
Err(BusySnapshot("database snapshot is stale, rollback and retry the transaction"))
Err(BusySnapshot("database snapshot is stale, rollback and retry the transaction"))
Err(Busy("database is locked"))
```

### Expected behavior

DDL on `aux` needs no write transaction on main, so all statements succeed. SQLite 3.50.4 (the same steps through rusqlite, journal_mode=WAL on both files, busy timeout 0; rusqlite's `execute` returns `changes()`, which the DDL leaves at 1) prints:

```
0
Ok(1)
Ok(1)
Ok(1)
Ok(1)
```

`CREATE TABLE aux.n(x)`, `DROP TABLE aux.u` and `ALTER TABLE aux.u RENAME TO u9` fail the same way; `EXPLAIN` shows a write `Transaction` on iDb=0 for DDL on `aux` (SQLite: only on `aux`). `translate_statement` calls `begin_write_operation()` for non-DML writes (`core/translate/mod.rs:569-578`), which always marks main as written (`core/vdbe/builder.rs:2185-2188`). Related, about the read transaction on main: #10813, #10825. The read-transaction side of attached schema changes: #11538. The same `begin_write_operation()` call also makes no-op `IF [NOT] EXISTS` DDL take the write lock: #11536.
