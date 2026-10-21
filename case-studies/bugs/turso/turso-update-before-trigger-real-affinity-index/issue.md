DRAFT, not filed
Title: UPDATE with a BEFORE UPDATE trigger writes an index key from a REAL column without REAL affinity, corrupting the index (SQLite-created file)

SQLite writes a whole-number value in a REAL column (`128.0`) to disk as an integer and turns it back into a REAL when it reads the column. In an UPDATE on a table with a BEFORE UPDATE trigger, Turso reads the columns again after the trigger, without that conversion. The new index key is computed from the integer `128`, while the key it deletes on the next UPDATE is computed from `128.0`. An expression index whose value depends on the type (`typeof`, `format`, `printf`, `quote`, ...) then gets a key that doesn't match the row.

```sh
sqlite3 r.db <<'SQL'
CREATE TABLE t(a REAL, b);
INSERT INTO t VALUES (128.0, 1);
CREATE INDEX i ON t(typeof(a), b);
CREATE TRIGGER g BEFORE UPDATE ON t BEGIN SELECT 1; END;
SQL
tursodb -q -m list r.db <<'SQL'
UPDATE t SET b = 2;
PRAGMA integrity_check;
UPDATE t SET b = 3;
SELECT a, b FROM t;
SQL
```

Turso:

```
row 1 missing from index i
Runtime error: Corrupt database: IdxDelete: no matching index entry found for key [Value(Text(Text { value: "real", subtype: Text })), Value(Numeric(Integer(2))), Value(Numeric(Integer(1)))] while seeking (11)
128.0|2
```

SQLite 3.50.4 (same SQL on a fresh copy of the file):

```
ok
128.0|3
```

After the first UPDATE, SQLite's `PRAGMA integrity_check` on the file Turso wrote also reports `row 1 missing from index i`. Without the trigger it works.

Related, same file without an index: in an AFTER UPDATE trigger, `typeof(OLD.a)` is `integer` in Turso, `real` in SQLite (`NEW.a` is `real` in both; in a DELETE trigger `OLD.a` is `real` in both).

Expected: `ok` and `128.0|3`.

Likely cause: in `core/translate/emitter/update.rs`, after the BEFORE triggers fire, the non-SET columns are read again with a second `emit_update_column_values` call (~1520). The column affinity applied to the NEW registers after the first read ("apply column affinity to the NEW values early", ~1358) isn't applied again, so the index key (and the row record) is built from the raw integer. The key for `IdxDelete` is built with `RealAffinity`. Same family as #9881 (hash join payload), different code path.

Tested on Turso main f8549ca2.
