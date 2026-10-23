Filed as #9932 (https://github.com/tursodatabase/turso/issues/9932) (line 1 corrected 2026-10-21).
Title: UPDATE through a covering index writes the row without finishing the deferred seek when a partial index WHERE reads only the rowid (panic or duplicate rowids)

```sql
CREATE TABLE t(id INTEGER PRIMARY KEY, a, b);
CREATE INDEX ia ON t(a);
CREATE INDEX ib ON t(b) WHERE id > 10;
INSERT INTO t VALUES (1, 1, 0);
UPDATE t SET b = 2 WHERE a = 1;
SELECT * FROM t;
```

Turso (`tursodb -q -m list`):

```
thread 'main' panicked at core/storage/btree.rs:8851:9:
self.current_page >= 0
```

(backtrace: `insert_into_page` <- `BTreeCursor::insert` <- `op_insert`)

SQLite 3.50.4:

```
1|1|2
```

When the table cursor was already positioned on an earlier row, there is no panic; the row is written at the wrong place instead, leaving a duplicate rowid:

```sql
CREATE TABLE t(id INTEGER PRIMARY KEY, a, b);
CREATE INDEX ia ON t(a);
CREATE INDEX ib ON t(b) WHERE id > 1;
INSERT INTO t VALUES (2, 1, 0), (1, 2, 0);
UPDATE t SET b = 9 WHERE a > 0;
SELECT * FROM t;
PRAGMA integrity_check;
```

Turso:

```
1|2|0
1|2|9
2|1|9
wrong # of entries in index ia
```

SQLite 3.50.4:

```
1|2|9
2|1|9
ok
```

SQLite's own `PRAGMA integrity_check` on the file Turso wrote reports `Tree 2 page 2 cell 0: Rowid 1 out of order` and `wrong # of entries in index ia`.

Expected: the SQLite output, no panic.

Conditions: the UPDATE scans a secondary index (`ia`) that, with the rowid, covers every column the UPDATE needs, so the table cursor is only moved by `DeferredSeek`; and a partial index (`ib`) contains an updated column and has a WHERE that reads only the rowid alias. Without the partial index, or with a partial WHERE on a non-rowid column, it works.

Likely cause: in `core/translate/emitter/update.rs` (~2347), `FinishSeek` is emitted before the table `Insert` only if no `Column` instruction on the table cursor appears in the loop body (`table_column_was_read`). Here the only such `Column` is the one reading the old `b` for the partial index's `IdxDelete`, and it sits behind `IfNot` on the old row's partial-index predicate. For a row outside the partial index that branch is skipped, `RowId` reads the deferred rowid without moving the cursor, and `Insert` runs on a cursor that was never moved to the row (unpositioned: the panic; still on the previous row: the duplicate). The check is static, but whether the read runs depends on the row.

Tested on Turso main f8549ca2.
