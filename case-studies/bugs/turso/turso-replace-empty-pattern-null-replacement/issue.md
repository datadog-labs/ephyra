Filed as #10426 (https://github.com/tursodatabase/turso/issues/10426) on 2026-10-21.
Title: replace(X, '', NULL) returns NULL instead of X

```sql
SELECT quote(replace('abc', '', NULL)), quote(replace(-1, '', NULL)), quote(replace('abc', x'', NULL)), quote(replace('abc', 'b', NULL));
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
NULL|NULL|NULL|NULL
```

SQLite 3.50.4 (trunk gives the same):
```
'abc'|'-1'|'abc'|NULL
```

Expected: when the pattern Y is an empty string, replace(X, Y, Z) returns X as text, even if Z is NULL. SQLite's `replaceFunc` returns as soon as it sees the empty pattern and never reads Z. Z is only checked when the pattern is not empty, so the last column is NULL in both engines. The docs say "If Y is an empty string then return X unchanged" (https://sqlite.org/lang_corefunc.html#replace). So a filter like `WHERE replace(c2, c0, NULL)` with c0 = '' and c2 = -1 keeps the row in SQLite and drops it in Turso.

Likely cause: `Value::exec_replace` (core/vdbe/value.rs) returns `Value::Null` when any of the three arguments is NULL, before its empty-pattern check. Checking the empty pattern before the NULL replacement (a NULL X or Y still gives NULL) would match SQLite.

Tested on Turso main db441331.
