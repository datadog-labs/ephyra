DRAFT, not filed (comment on existing issue #7644, not a new issue)
Title: (comment on #7644) The TEMP trigger also blocks RENAME COLUMN on unrelated tables

The temp trigger doesn't have to be on the table being altered. Once any TEMP trigger exists on a main table, RENAME COLUMN fails on every table:

```sql
CREATE TABLE t0(a);
CREATE TABLE t1(b);
CREATE TEMP TRIGGER w0 AFTER DELETE ON t0 BEGIN SELECT 1; END;
ALTER TABLE t1 RENAME COLUMN b TO b2;
SELECT name FROM pragma_table_info('t1');
```

Turso:
```
Parse error: error in trigger w0 after rename column: trigger table not found: t0
b
```
SQLite 3.50.4:
```
b2
```

Tested on Turso main cbd5ac4e.
