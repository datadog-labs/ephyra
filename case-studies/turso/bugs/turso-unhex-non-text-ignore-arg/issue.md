Filed as #10400 (https://github.com/tursodatabase/turso/issues/10400) on 2026-10-21.
Title: unhex(X, Y) returns NULL when Y is an INTEGER, REAL or BLOB instead of using its text

```sql
SELECT quote(unhex('12-34', -1)), quote(unhex('1234', 0)), quote(unhex('12.34', 0.5)), quote(unhex('12 34', x'20'));
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
NULL|NULL|NULL|NULL
```

SQLite 3.50.4 (trunk gives the same):
```
X'1234'|X'1234'|X'1234'|X'1234'
```

Expected: SQLite reads the second argument with `sqlite3_value_text()`, so a number or a BLOB gives the set of characters to skip just like a TEXT value does (`-1` allows `-`, `0.5` allows `.`, `x'20'` allows a space). Only a NULL second argument gives NULL (https://sqlite.org/lang_corefunc.html#unhex). The same happens when the second argument is a column or expression of a numeric type, e.g. `unhex(c, c IN (SELECT ...))`, or `unhex(c, c)` with c = -17 (SQLite X'17').

Likely cause: in `Value::exec_unhex` (core/vdbe/value.rs) the two-argument branch only handles `Value::Text(_)` for the ignore set; every other type falls into `_ => Value::Null`. Converting the argument to text first (NULL stays NULL) would match SQLite.

Tested on Turso main db441331 (also cbd5ac4e).
