# A statement that got Busy on an attached database keeps returning Busy after the lock is released, until it is reset

Posted as https://github.com/tursodatabase/turso/issues/11546 on 2026-11-06. The text below is the issue as posted.

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
    a.execute("CREATE TABLE aux.u(x)", ()).await.unwrap();

    a.execute("BEGIN", ()).await.unwrap();
    a.execute("INSERT INTO aux.u VALUES (1)", ()).await.unwrap();

    let mut ins = b.prepare("INSERT INTO aux.u VALUES (2)").await.unwrap();
    let mut rows = ins.query(()).await.unwrap();
    println!("{:?}", rows.next().await);

    a.execute("COMMIT", ()).await.unwrap();

    println!("{:?}", rows.next().await);
    println!("{:?}", rows.next().await);

    drop(rows);
    ins.reset().unwrap();
    println!("{:?}", ins.execute(()).await);
}
```

### Current behavior

After A's `COMMIT`, every retry of B's INSERT still returns Busy; only a reset lets it run:

```
Err(Busy("database is locked"))
Err(Busy("database is locked"))
Err(Busy("database is locked"))
Ok(1)
```

### Expected behavior

The first retry after `COMMIT` runs the INSERT. SQLite 3.50.4 (the same steps through rusqlite, journal_mode=WAL on both files, busy timeout 0, raw `sqlite3_step` for the retry, then `SELECT count(*) FROM aux.u`) prints:

```
SQLITE_BUSY
SQLITE_DONE
count: 2
```

With a table in the main database instead of `aux.u`, Turso's first retry succeeds. For the attached database the failed `begin_write_tx` leaves the read transaction open (`core/vdbe/execute.rs:5025-5027`), so each retry hits `BusySnapshot` on the stale snapshot, which `fail_step` reports as Busy (`core/vdbe/mod.rs:2974-2980`); for main the read transaction is ended (`core/vdbe/execute.rs:4966-4987`).
