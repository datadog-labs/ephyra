**Title:** LEFT JOIN with `x = NULL AND y IN (…)` on a two-column index returns a matched row as well as the NULL-extended one (regression in 3.43.0)

```sql
CREATE TABLE t(a, b, c);
INSERT INTO t VALUES (1, NULL, 5), (2, NULL, NULL);
CREATE INDEX i ON t(b, c);
SELECT x.a, y.a FROM t AS x LEFT JOION t AS y ON y.b = x.c AND y.c IN (5, 0);
```

Output from trunk (check-in 2662a68f0d5c, 2026-10-07), built from the source tarball with default options:

```
1|
2|
2|1
```

Expected `1|` and `2|` only, which is what `t AS y NOT INDEXED` returns. For `x.a = 2`, `x.c` is NULL, so `y.b = x.c` is never true and the row can only be NULL-extended. The indexed plan (`SEARCH y USING INDEX i (b=? AND c=?) LEFT-JOIN`) returns it NULL-extended and also matched to `y.a = 1`.

It needs an IN list of at least two distinct numeric values, one of which matches `y.c`:
- these are wrong: `IN (5, 0)`, `IN (0, 5)`, `IN (1, 5)`, `IN (0.0, 5)`;
- these are correct: `IN (5)`, `IN (5, 5)`, `IN (0, 6)`, `IN ('', 5)`, `IN (NULL, 5)`, and `y.c = 5`;
- the same query as an inner join is correct.

**Versions** (official amalgamations, default options):

| Versions | Result |
|---|---|
| through 3.24.0 | correct |
| 3.25.0 to 3.37.0 | wrong |
| 3.38.0 to 3.42.0 | correct |
| 3.43.0, 3.44.0, 3.45.0, 3.53.4, the macOS system 3.51.0 and trunk | wrong |

So it looks like a regression in 3.25.0, fixed in 3.38.0, and back in 3.43.0.

We found this with a property-based test generator for SQLite databases and queries, using SQLancer-style TLP and NoREC oracles: partitioning the WHERE of the join, and the NoREC rewrite, both disagreed with the original query.
