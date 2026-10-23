Filed as #10397 (https://github.com/tursodatabase/turso/issues/10397) on 2026-10-21.
Title: PRAGMA table_info / table_list keep a view's old column list after its table changes (ADD COLUMN, DROP TABLE) until the database is reopened

```sql
CREATE TABLE t(a, b);
CREATE VIEW v AS SELECT * FROM t;
CREATE VIEW w AS SELECT 1 FROM t;
ALTER TABLE t ADD COLUMN c;
SELECT name FROM pragma_table_info('v');
SELECT name, ncol FROM pragma_table_list WHERE type = 'view' ORDER BY name;
DROP TABLE t;
SELECT name, ncol FROM pragma_table_list WHERE type = 'view' ORDER BY name;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
a
b
v|2
w|1
v|2
w|1
```

SQLite 3.50.4 (trunk gives the same):
```
a
b
c
v|3
w|1
v|0
w|0
```

Expected: `v` is `SELECT *`, so after the ADD COLUMN it has three columns, as SQLite reports and as `SELECT * FROM v` already returns in Turso. After `DROP TABLE t` neither view can be resolved and SQLite reports `ncol` 0. Turso keeps the column list computed when the view was created; closing and reopening the database gives the right answer (`a b c`, `v|3`), so only the in-memory schema is stale.

Likely cause: the view's column list is resolved once at CREATE VIEW / schema load and not recomputed when a table it reads is altered or dropped.

Tested on Turso main db441331.
