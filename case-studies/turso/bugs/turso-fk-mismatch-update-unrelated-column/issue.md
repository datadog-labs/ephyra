Filed as #10401 (https://github.com/tursodatabase/turso/issues/10401) on 2026-10-21.
Title: UPDATE of a child table fails with "foreign key mismatch" even when no foreign key column is assigned

```sql
CREATE TABLE p(a, b);
CREATE TABLE c(x, y REFERENCES p(b));
INSERT INTO c VALUES (1, NULL);
PRAGMA foreign_keys = 1;
UPDATE c SET x = 2;
SELECT * FROM c;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
  × foreign key mismatch - "c" referencing "p"

1|
```

SQLite 3.50.4 (trunk gives the same):
```
2|
```

Expected: the UPDATE succeeds. `p(b)` has no UNIQUE index, so the foreign key is a mismatch, and SQLite reports that error only when a statement has to check the key: an INSERT into `c`, a DELETE from `c`, or an UPDATE that assigns `c.y` (see "foreign key DML errors" in https://sqlite.org/foreignkeys.html#fk_indexes). An UPDATE that only sets `x` never looks up the parent, so SQLite runs it. Turso fails it, and the same happens with `UPDATE ... FROM`.

Likely cause: `emit_fk_child_update_counters` (core/translate/fkeys.rs) calls `resolved_fks_for_child`, which resolves every foreign key of the child up front and returns `fk_mismatch_err` (core/schema.rs `resolve_fk`) for the first one without a parent index. The `child_key_changed(updated_cols, ...)` check that skips unchanged keys only runs after that, inside the loop. Resolving each key after the `child_key_changed` check, or skipping the keys whose columns are not assigned before resolving them, would match SQLite. Parent-side updates (`UPDATE p SET a = 2`) already succeed in both.

Tested on Turso main db441331 (also cbd5ac4e).
