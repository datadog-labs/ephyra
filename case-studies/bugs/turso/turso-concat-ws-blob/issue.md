Filed as #10394 (https://github.com/tursodatabase/turso/issues/10394) on 2026-10-21.
Title: concat_ws() drops BLOB arguments and returns NULL when the separator is a BLOB

```sql
SELECT quote(concat_ws(',', 'a', x'62', 'c'));
SELECT quote(concat_ws(x'2c', 'a', 'b'));
SELECT quote(concat_ws(x'', NULL));
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
'a,c'
NULL
NULL
```

SQLite 3.50.4 (trunk gives the same):
```
'a,b,c'
'a,b'
''
```

Expected: SQLite's concat_ws() reads every non-NULL argument, BLOBs included, as text, and returns NULL only when the separator is NULL (https://sqlite.org/lang_corefunc.html#concat_ws). `concat()` already handles BLOBs this way in Turso: `concat(x'61', 'b')` is `'ab'`.

Likely cause: `Value::exec_concat_ws` in core/vdbe/value.rs returns `Value::Null` for `Value::Blob(_)` in the separator match, and its `filter_map` keeps only `Text` and `Numeric` arguments, so BLOB arguments are skipped like NULLs.

Tested on Turso main db441331.
