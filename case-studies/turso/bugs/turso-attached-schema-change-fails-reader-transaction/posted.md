# "Database schema changed" in a read transaction after another connection changes the schema of an attached database

Posted as https://github.com/tursodatabase/turso/issues/11538 on 2026-11-06. The text below is the issue as posted.

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
```

### Current behavior

After B's ALTER TABLE commits, A's next read of `aux` inside its transaction fails (every statement that reads `aux`, including `aux.sqlite_schema`, fails until the transaction ends):

```
Integer(1)
Err(Error("Database schema changed"))
Ok(Some((Integer(1), Null)))
```

### Expected behavior

A's transaction keeps reading its snapshot. SQLite 3.50.4 (the same steps through rusqlite, journal_mode=WAL on both files, plus a read of `aux.sqlite_schema` inside the transaction) prints:

```
1
SELECT * FROM aux.u: [Integer(1)]
aux.sqlite_schema: ["u"]
after COMMIT: [Integer(1), Null]
```

When B changes the schema of main instead, Turso reads the old columns from A's snapshot like SQLite. For attached databases the catalog takes the schema from the shared `Database` object (`core/translate/catalog.rs:236-243`), where B's commit has already published the new one, so every re-prepare ends in `SchemaUpdated`. The write side of DDL on attached databases: #11539. The same mismatch for main after `ROLLBACK TO`: #11537. Possibly related: #5930 (closed), #10975 (open).
