# UPDATE that sets the rowid of an extension virtual table panics "NotExists on unexpected cursor"

Posted as https://github.com/tursodatabase/turso/issues/11545 on 2026-11-06. The text below is the issue as posted.

---

### Reproduction

Turso main 2709799da, `tursodb -q -m list :memory: < repro.sql` from the turso checkout, with Turso's own test extension `kv_store` (`cargo build -p turso_ext_tests`).

```sql
.load target/release/libturso_ext_tests
CREATE VIRTUAL TABLE t USING kv_store;
INSERT INTO t(key, value) VALUES ('k', 'v');
UPDATE t SET rowid = rowid + 1;
```

### Current behavior

The UPDATE panics before the module's update callback is called:

```
thread 'main' panicked at core/vdbe/execute.rs:14270:18:
NotExists on unexpected cursor
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

### Expected behavior

The row moves to the new rowid. SQLite 3.50.4 with an equivalent module written against `sqlite3_module` (`memvt`, columns a, b; `[memvt]` lines are its hooks) calls `xUpdate` with the old and the new rowid (the same statements with a, b for key, value, plus a final `SELECT rowid, a, b FROM t`):

```
SQLite 3.50.4
a> CREATE VIRTUAL TABLE t USING memvt;
  [memvt] create
  [memvt] commit
a> INSERT INTO t(a, b) VALUES ('k', 'v');
  [memvt] begin
  [memvt] insert rowid=1 a=k b=v
  [memvt] commit
a> UPDATE t SET rowid = rowid + 1;
  [memvt] begin
  [memvt] update rowid=1 new_rowid=2 a=k b=v
  [memvt] commit
a> SELECT rowid, a, b FROM t;
  2|k|v
```

Updates that do not set the rowid work. The UPDATE emits a `NotExists` on the `VOpen` cursor (`core/translate/emitter/update.rs:1279-1284`), and `op_not_exists` accepts only B-tree cursors (`core/vdbe/execute.rs:14270`). A fix may also need `turso_ext`: the derived `update` drops the new rowid (`macros/src/ext/vtab_derive.rs:171-177`).
