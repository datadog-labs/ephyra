# Writing an extension virtual table from a connection other than the one that created it panics "Could not find virtual table for VBegin"; DROP TABLE from it fails

Posted as https://github.com/tursodatabase/turso/issues/11544 on 2026-11-06. The text below is the issue as posted.

---

### Reproduction

Turso main 52d6fd81d, with `turso_core` (a host is needed because tursodb has one connection) and Turso's own test extension `kv_store` (`cargo build -p turso_ext_tests`) loaded with `Connection::load_extension`, which tursodb's `.load` also calls; run from the turso checkout.

```rust
use std::sync::Arc;
use turso_core::{ConnectOpts, Connection, Database, OpenOptions, PlatformIO, StepResult};

// extensions are built with mimalloc (extensions/core/README.md); the host must use it too
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn exec(c: &Arc<Connection>, sql: &str) {
    let mut st = c.prepare(sql).unwrap();
    loop {
        match st.step() {
            Ok(StepResult::Row) => {
                let r: Vec<String> = st.row().unwrap().get_values().map(|v| v.to_string()).collect();
                println!("{}", r.join("|"));
            }
            Ok(StepResult::IO) => st._io().step().unwrap(),
            Ok(_) => break,
            Err(e) => { println!("error: {e}"); break; }
        }
    }
}

fn main() {
    let ext = "target/release/libturso_ext_tests.dylib";
    let io = Arc::new(PlatformIO::new().unwrap());
    let db = Database::open(io, "t.db", OpenOptions::new(Arc::new(turso_sqlite::SqliteDialect))).unwrap();
    let connect = || { let c = db.connect(ConnectOpts::new()).unwrap(); c.load_extension(ext).unwrap(); c };

    let a = connect();
    exec(&a, "CREATE VIRTUAL TABLE v USING kv_store");
    exec(&a, "INSERT INTO v(key, value) VALUES ('k1', 'x')");

    let b = connect();
    exec(&b, "SELECT key, value FROM v");
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        exec(&b, "INSERT INTO v(key, value) VALUES ('k2', 'y')")
    }));

    let c = connect();
    exec(&c, "DROP TABLE v");
    exec(&c, "SELECT name FROM sqlite_schema");
}
```

### Current behavior

Connection `b` reads the row, but its INSERT panics, dropping the failed statement panics again, and `DROP TABLE v` from `c` fails and leaves the table in `sqlite_schema` (thread ids, checkout path, the RUST_BACKTRACE note and the backtrace removed):

```
k1|x

thread 'main' panicked at core/vdbe/execute.rs:1731:14:
Could not find virtual table for VBegin

thread 'main' panicked at core/vdbe/execute.rs:4189:14:
vtab must exist
error: Internal error: Could not find Virtual Table to Destroy
v
```

### Expected behavior

Any connection that loaded the module can write and drop the table. SQLite 3.50.4 with an equivalent module written against `sqlite3_module` (`memvt`, columns a, b, rows shared by all connections, registered on each connection; `[memvt]` lines are its hooks) runs the same steps, plus a SELECT on `b` after its INSERT:

```
SQLite 3.50.4
a> CREATE VIRTUAL TABLE v USING memvt;
  [memvt] create
  [memvt] commit
a> INSERT INTO v(a, b) VALUES (1, 'x');
  [memvt] begin
  [memvt] insert rowid=1 a=1 b=x
  [memvt] commit
b> SELECT rowid, a, b FROM v;
  1|1|x
b> INSERT INTO v(a, b) VALUES (2, 'y');
  [memvt] begin
  [memvt] insert rowid=2 a=2 b=y
  [memvt] commit
b> SELECT rowid, a, b FROM v;
  1|1|x
  2|2|y
c> DROP TABLE v;
  [memvt] destroy
a> SELECT name FROM sqlite_schema;
```

It makes no difference whether `b` connects before or after the `CREATE VIRTUAL TABLE`. `op_vcreate` registers the table only in the creating connection's symbol table (`core/vdbe/execute.rs:1482-1486`), and `op_vbegin`, `vtab_rollback_all` and `op_vdestroy` look it up in the current connection's (`core/vdbe/execute.rs:1727-1731`, `4184-4189`, `1698`).
