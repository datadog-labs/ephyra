Filed as #10339 (https://github.com/tursodatabase/turso/issues/10339) on 2026-10-21.
Title: pragma_quick_check / pragma_integrity_check name their column "message" instead of "quick_check" / "integrity_check"

```sql
SELECT quick_check FROM pragma_quick_check;
SELECT integrity_check FROM pragma_integrity_check;
```

Turso:
```
Parse error: no such column: quick_check
Parse error: no such column: integrity_check
```
SQLite 3.50.4 (and trunk):
```
ok
ok
```

Expected: the table-valued forms use the same column name as the PRAGMA statement (`PRAGMA quick_check` already reports a column named `quick_check` in Turso). `SELECT * FROM pragma_quick_check` in SQLite has the header `quick_check`.

Likely cause: in `core/pragma.rs` the `IntegrityCheck` and `QuickCheck` entries declare their result column as `"message"`, while `core/translate/integrity_check.rs` names the statement's column `quick_check` / `integrity_check`.

Tested on Turso main cbd5ac4e.
