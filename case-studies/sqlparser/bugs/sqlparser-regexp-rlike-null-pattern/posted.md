# MySQL: `REGEXP`/`RLIKE` followed by `NULL` is rejected or misparsed (regression in 0.59.0)

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2760 on 2026-10-28. The text below is the issue as posted.

---

Since 0.59.0, a `NULL` pattern after `REGEXP` or `RLIKE` is consumed as if it were `NOT NULL`, and the next token becomes the pattern:

```
SELECT 'abc' REGEXP NULL
  -> sql parser error: Expected: an expression, found: EOF

SELECT a RLIKE NULL FROM t
  -> displays as: SELECT a RLIKE FROM AS t      (pattern is the identifier FROM, no FROM clause)

SELECT a NOT REGEXP NULL OR b FROM t
  -> displays as: SELECT a NOT REGEXP OR AS b FROM t
```

In 0.58.0 these parsed as `Expr::RLike { pattern: Value(Null), .. }` and round-tripped.

Tested on main e15e9abf (also 0.63.0; 0.58.0 is fine).

Found with a property-based fuzzer (round trip and statement-separation checks); I used AI assistance for triage and verified every case above.
