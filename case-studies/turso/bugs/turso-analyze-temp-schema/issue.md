Filed as #10340 (https://github.com/tursodatabase/turso/issues/10340) on 2026-10-21.
Title: ANALYZE cannot reach TEMP objects: "ANALYZE temp", "ANALYZE <temp table>" and "ANALYZE <temp index>" fail

```sql
CREATE TEMP TABLE tt(a);
CREATE INDEX tt_i ON tt(a);
INSERT INTO tt VALUES (1), (2);
ANALYZE temp;
ANALYZE tt;
ANALYZE tt_i;
ANALYZE temp.tt;
SELECT tbl, idx, stat FROM temp.sqlite_stat1;
```

Turso:
```
Parse error: no such table or index: "temp"
Parse error: no such table or index: tt
Parse error: no such table or index: tt_i
tt|tt_i|2 1
```
SQLite 3.50.4 (and trunk):
```
tt|tt_i|2 1
```

Expected: `ANALYZE temp` analyzes the temp schema, and an unqualified name finds a temp table or index the same way other statements do. Only the qualified form `ANALYZE temp.tt` works now.

Likely cause: `resolve_analyze_targets` in `core/translate/analyze.rs` excludes `TEMP_DB_ID` when checking whether the name is a schema name, and looks up unqualified table/index names only in main (database 0).

Tested on Turso main cbd5ac4e.
