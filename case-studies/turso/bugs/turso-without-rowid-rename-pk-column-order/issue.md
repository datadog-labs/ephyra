Filed as #10387 (https://github.com/tursodatabase/turso/issues/10387) on 2026-10-21.
Title: RENAME COLUMN of a WITHOUT ROWID table's primary-key column moves it to the end of the key: reads return permuted values and INSERT writes permuted rows

```sql
CREATE TABLE t(a, b, c, PRIMARY KEY(a, b, c)) WITHOUT ROWID;
INSERT INTO t VALUES (1, 2, 3);
ALTER TABLE t RENAME COLUMN a TO x;
SELECT * FROM t;
SELECT count(*) FROM t WHERE x = 1;
INSERT INTO t VALUES (4, 5, 6);
SELECT * FROM t;
```

Turso (`tursodb -q -m list --experimental-without-rowid test.db < repro.sql`):
```
3|1|2
0
3|1|2
4|5|6
```

SQLite 3.50.4 (trunk gives the same):
```
1|2|3
1
1|2|3
4|5|6
```

Then `sqlite3 test.db "SELECT * FROM t; PRAGMA integrity_check"` on the file Turso wrote:
```
1|2|3
5|6|4
ok
```

Expected: renaming a column changes nothing but its name. After the rename Turso reads the key as if the renamed column came last (`b, c, x`), so `SELECT *` returns `3|1|2`, `WHERE x = 1` finds nothing, and the row inserted after the rename is stored with its values rotated: SQLite, and Turso after a reopen, read it back as `5|6|4`. The integrity check passes because the permuted record is still a valid key, so the damage is silent. Renaming the last key column, or a column that is not in the key, is fine; with `PRIMARY KEY(a, b)` renaming `a` swaps `a` and `b`. The same happens on a WITHOUT ROWID table in a file created by SQLite, without `--experimental-without-rowid`. Reopening the database reads correctly until the next rename.

Likely cause: the in-memory schema update for RENAME COLUMN (around "Keep primary_key_columns consistent" in core/vdbe/execute.rs) leaves the WITHOUT ROWID key-column order out of sync with the renamed column, so the renamed key column is placed after the others.

Tested on Turso main db441331.
