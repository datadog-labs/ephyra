# changes() / total_changes() differ from SQLite: CREATE INDEX resets changes(), INSERT OR FAIL inside BEGIN, and virtual-table writes

Posted as https://github.com/tursodatabase/turso/issues/11547 on 2026-11-06 (one issue for turso-create-index-resets-changes, turso-total-changes-or-fail-in-transaction and turso-vtab-writes-not-counted-in-changes; this bug is its item 1). The text below is the issue as posted.

---

### Reproduction

Turso main 2709799da, `tursodb -q -m list :memory: < repro.sql` (SQLite: `sqlite3 :memory: < repro.sql`).

```sql
CREATE TABLE t(x);
INSERT INTO t VALUES (1), (2), (3);
SELECT changes();
CREATE INDEX i ON t(x);
SELECT changes();
CREATE TABLE u(a UNIQUE);
INSERT INTO u VALUES (1);
BEGIN;
INSERT OR FAIL INTO u VALUES (2), (3), (1);
SELECT changes(), total_changes();
COMMIT;
```

And for virtual tables, with Turso's test extension `kv_store` (`cargo build -p turso_ext_tests`), run from the turso checkout:

```sql
.load target/release/libturso_ext_tests
CREATE TABLE t(x);
INSERT INTO t(rowid, x) VALUES (100, 1);
CREATE VIRTUAL TABLE v USING kv_store;
INSERT INTO v(key, value) VALUES ('k1', 'x');
SELECT changes(), total_changes(), last_insert_rowid();
UPDATE v SET value = 'y';
SELECT changes(), total_changes(), last_insert_rowid();
DELETE FROM v;
SELECT changes(), total_changes(), last_insert_rowid();
```

### Current behavior

1. `CREATE INDEX` sets `changes()` to 0 (second line).
2. The rows 2 and 3 kept by the failed `INSERT OR FAIL` inside `BEGIN` are counted in `changes()` but not in `total_changes()` (last line):

```
3
0
Runtime error: UNIQUE constraint failed: u.a (19)
2|4
```

3. Virtual-table INSERT, UPDATE and DELETE are not counted, and the INSERT does not set `last_insert_rowid()`:

```
0|1|100
0|1|100
0|1|100
```

### Expected behavior

SQLite 3.50.4 leaves `changes()` alone on CREATE INDEX and adds the kept rows to `total_changes()`:

```
3
3
Runtime error near line 9: UNIQUE constraint failed: u.a (19)
2|6
```

and counts virtual-table rows like table rows. With an equivalent module written against `sqlite3_module` (`memvt`, columns a, b; the same statements with a, b for key, value), the three SELECTs give:

```
1|2|1
1|3|1
1|4|1
```

`CREATE INDEX IF NOT EXISTS` on an existing index also resets `changes()`; other DDL leaves it alone, and without `BEGIN`/`COMMIT` Turso's `total_changes()` is 6 as in SQLite. Causes: CREATE INDEX is in `change_cnt_on` (`core/translate/mod.rs:91-97`); `Program::abort` calls `set_changes` but not `add_total_changes` for the FAIL case (`core/vdbe/mod.rs:3819-3914`); `op_vupdate` never calls `record_statement_change()` and sets the last rowid only for REPLACE (`core/vdbe/execute.rs:1636-1649`).
