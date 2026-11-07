# CREATE ... IF NOT EXISTS / DROP ... IF EXISTS that does nothing fails with "database is locked" while another connection writes

Posted as https://github.com/tursodatabase/turso/issues/11536 on 2026-11-06. The text below is the issue as posted.

---

### Reproduction

Turso main 52d6fd81d, with the `turso` crate (tokio), file database, default options.

```rust
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
```

### Current behavior

All four statements change nothing, but each needs the write lock and fails:

```
CREATE TABLE IF NOT EXISTS t(x): Err(Busy("database is locked"))
CREATE INDEX IF NOT EXISTS i ON t(x): Err(Busy("database is locked"))
DROP TABLE IF EXISTS nosuch: Err(Busy("database is locked"))
DROP INDEX IF EXISTS nosuch: Err(Busy("database is locked"))
```

### Expected behavior

They run with only a read transaction. SQLite 3.50.4 (the same steps through rusqlite, journal_mode=WAL, busy timeout 0) prints:

```
CREATE TABLE IF NOT EXISTS t(x): Ok(0)
CREATE INDEX IF NOT EXISTS i ON t(x): Ok(0)
DROP TABLE IF EXISTS nosuch: Ok(0)
DROP INDEX IF EXISTS nosuch: Ok(0)
```

`EXPLAIN CREATE TABLE IF NOT EXISTS t(x)` shows `Transaction ... tx_mode=Write` in Turso and a read `Transaction 0 0` in SQLite. `translate_inner` requests a write transaction for every CREATE/DROP by statement kind (`core/translate/mod.rs:569-578`), even when the translator returns early for the no-op case.
