Filed as #10355 (https://github.com/tursodatabase/turso/issues/10355) on 2026-10-21.
Title: ANALYZE on a database without tables does not create sqlite_stat1

```sql
ANALYZE;
SELECT name FROM sqlite_schema;
```

Turso (`tursodb -q -m list :memory: < repro.sql`): no output.

SQLite 3.50.4 (trunk gives the same):
```
sqlite_stat1
```

Expected: `ANALYZE` creates `sqlite_stat1` even when there is nothing to analyze, as SQLite does. Turso creates it only when there is at least one table (`CREATE TABLE t(a); ANALYZE;` creates it). The difference shows up in `sqlite_schema`, `pragma_table_list`, and in later statements that read or write `sqlite_stat1` (for example `INSERT INTO sqlite_stat1 ...` after `ANALYZE` fails with "no such table"). A database with only views behaves the same way.

Tested on Turso main cbd5ac4e.
