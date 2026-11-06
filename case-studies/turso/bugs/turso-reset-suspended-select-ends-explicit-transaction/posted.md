# Resetting a SELECT that was started before BEGIN rolls back the explicit transaction

Posted as https://github.com/tursodatabase/turso/issues/11542 on 2026-11-06. The text below is the issue as posted.

---

### Reproduction

Turso main 52d6fd81d, with the `turso` crate (tokio), file database.

```rust
use turso::Builder;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db = Builder::new_local("t.db").build().await.unwrap();
    let c = db.connect().unwrap();
    c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1), (2);").await.unwrap();

    let mut s = c.prepare("SELECT x FROM t").await.unwrap();
    let mut rows = s.query(()).await.unwrap();
    println!("{:?}", rows.next().await.unwrap().unwrap().get_value(0).unwrap());

    c.execute("BEGIN", ()).await.unwrap();
    c.execute("INSERT INTO t VALUES (3)", ()).await.unwrap();
    println!("autocommit: {}", c.is_autocommit().unwrap());

    drop(rows);
    s.reset().unwrap();
    println!("autocommit: {}", c.is_autocommit().unwrap());

    let mut n = c.query("SELECT count(*) FROM t", ()).await.unwrap();
    println!("{:?}", n.next().await.unwrap().unwrap().get_value(0).unwrap());
    drop(n);

    println!("{:?}", c.execute("COMMIT", ()).await);
}
```

### Current behavior

The reset silently rolls back the transaction: the connection is back in autocommit mode, the INSERT is gone and `COMMIT` fails:

```
Integer(1)
autocommit: false
autocommit: true
Integer(2)
Err(Error("cannot commit - no transaction is active"))
```

### Expected behavior

The transaction stays open and `COMMIT` keeps the row. SQLite 3.50.4 (the same steps through rusqlite) prints:

```
Integer(1)
autocommit: false
autocommit: false
Integer(3)
Ok(1)
```

With `SAVEPOINT sp` instead of `BEGIN`, the savepoint is gone after the reset (`ROLLBACK TO sp` / `RELEASE sp` fail with "no such savepoint: sp"); stepping the SELECT to the end instead of resetting it works. The rollback comes from `Program::abort` (`core/vdbe/mod.rs:3925-3930`): `can_autocommit_now` (`core/vdbe/mod.rs:1605-1609`) does not check that the connection is still in autocommit mode. Related but different: #7260 / #7345 (there BEGIN came before the SELECT).
