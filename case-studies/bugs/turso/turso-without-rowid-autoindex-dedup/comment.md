DRAFT, not filed — comment for #9081

It isn't only `count(*)`: any join into a WITHOUT ROWID table through an automatic index loses rows when the query reads none of that table's primary-key columns. A file made by Turso shows it:

```sql
CREATE TABLE w(k PRIMARY KEY, v) WITHOUT ROWID;
INSERT INTO w VALUES('a',1),('b',1);
CREATE TABLE t(x);
INSERT INTO t VALUES(1);
SELECT t.x FROM t JOIN w ON w.v = t.x;               -- Turso: 1          SQLite: 1, 1
SELECT count(*) OVER () FROM t JOIN w ON w.v = t.x;  -- Turso: 1          SQLite: 2, 2
SELECT w.k FROM t JOIN w ON w.v = t.x;               -- both: a, b
```

(main f8549ca2, `--experimental-without-rowid`, against SQLite 3.50.4.)

Likely cause: `ephemeral_index_build` (`core/translate/optimizer/mod.rs`) puts only the columns the query uses into the automatic index, and `emit_autoindex` (`core/translate/main_loop/close.rs`) appends the rowid only when the table has one (`table_has_rowid`). For a WITHOUT ROWID table nothing makes the entries unique, so rows with the same used columns become one index entry. The primary-key columns would serve as the rowid does.
