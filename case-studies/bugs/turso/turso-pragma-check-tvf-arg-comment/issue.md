DRAFT, not filed
Title: (comment on #10339) pragma_quick_check / pragma_integrity_check also reject their table-name argument

Comment text for #10339:

The same two entries in core/pragma.rs also leave out the argument. SQLite declares `integrity_check` and `quick_check` with `PragFlg_Result1|PragFlg_SchemaOpt`, so the table-valued forms take the optional table name (the hidden `arg` column) and a schema, like `PRAGMA quick_check(t)` does. Turso's entries have only `NeedSchema | ReadOnly | Result0`, so the argument is rejected:

```sql
CREATE TABLE t(a);
SELECT * FROM pragma_quick_check('t');
SELECT * FROM pragma_integrity_check('t');
```

Turso:
```
Parse error: Too many arguments for pragma_quick_check: expected at most 0, got 1
Parse error: Too many arguments for pragma_integrity_check: expected at most 0, got 1
```

SQLite 3.50.4 (and trunk):
```
ok
ok
```

`PRAGMA quick_check('t')` and `PRAGMA integrity_check(t)` already work in Turso.

Tested on Turso main cbd5ac4e.
