DRAFT, not filed (comment on existing issue #9087, not a new issue)
Title: (comment on #9087) The later row-value components aren't evaluated once an earlier one is NULL, so the result uses the previous row's values

Comment text for #9087:

The cause looks like the LHS loop in `translate_subquery_result_expr` (core/translate/expr/translator.rs, `SubqueryType::In`). It emits `IsNull -> label_null_rewind` right after each component is computed, so when component i is NULL, components i+1.. are never computed for that row. The NULL-check scan then compares the RHS rows against whatever those registers held before: NULL on the first row (the NULL result in the report above), or the previous outer row's values. That second case also gives wrong rows the other way, true where SQLite gives NULL:

```sql
CREATE TABLE t(id INTEGER PRIMARY KEY, a, b);
INSERT INTO t VALUES (1, 5, 5), (2, NULL, 8), (3, 1, 2), (4, NULL, 2);
CREATE TABLE u(a, b);
INSERT INTO u VALUES (7, 8), (1, 2);
SELECT id, quote((a, b) NOT IN (SELECT a, b FROM u)) FROM t;
SELECT id FROM t WHERE (a, b) NOT IN (SELECT a, b FROM u);
```

Turso:
```
1|1
2|1
3|0
4|NULL
1
2
```

SQLite 3.50.4 (and trunk):
```
1|1
2|NULL
3|0
4|NULL
1
```

Row 2 is `(NULL, 8)`. `(7, 8)` could match it, so the result is NULL. Turso compares `(NULL, 5)` instead, with b still holding row 1's 5, finds nothing that could match and returns true. Row 4 comes out right only because row 3 left b = 2. Computing all components before the NULL checks should fix both cases.

Tested on Turso main db441331.
