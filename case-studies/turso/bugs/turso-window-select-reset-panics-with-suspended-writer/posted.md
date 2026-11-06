# Panic "shared autocommit transaction needed full rollback outside MVCC" when a window-function SELECT is reset while an INSERT ... RETURNING is suspended

Posted as https://github.com/tursodatabase/turso/issues/11543 on 2026-11-06. The text below is the issue as posted.

---

### Reproduction

Turso main 52d6fd81d, with the `turso` crate (tokio), release build.

```rust
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
```

### Current behavior

`win.reset()` panics (thread id, checkout path and the RUST_BACKTRACE note removed):

```
Some(Integer(1))
Some(Integer(3))

thread 'main' panicked at core/vdbe/mod.rs:3723:17:
shared autocommit transaction needed full rollback outside MVCC
```

### Expected behavior

The reset succeeds and the INSERT finishes. SQLite 3.50.4 (the same steps through the C API, plus a final `SELECT count(*) FROM t`) prints:

```
Some(Integer(1))
Some(Integer(3))
Ok(())
Ok(false)
count: 3
```

With a plain `SELECT id, a FROM t ORDER BY id` instead of the window SELECT there is no panic. The window SELECT has `n_change` 2 at the reset because the window buffer is filled with `Insn::Insert` without `skip_all_change_counts()` / `skip_last_rowid()` (`core/translate/window.rs:2107-2113`); the same missing flags cause #8698 (a window function overwrites `last_insert_rowid()`).
