**Title:** A UNION ALL subquery in FROM with a RIGHT JOIN after its first SELECT loses WHERE terms when flattened

```sql
CREATE TABLE t0(c0);
CREATE TABLE t1(c0);
INSERT INTO t0 VALUES (NULL);
SELECT * FROM (SELECT 5 FROM t0 WHERE (SELECT 0) UNION ALL SELECT 7 FROM t1 RIGHT JOIN t0 ON 0);
SELECT * FROM (SELECT c0 FROM t0 UNION ALL SELECT a.c0 FROM t0 AS a RIGHT JOIN t0 AS b ON 0) WHERE abs(c0);
```

Output from trunk (check-in 07095b76c1f0, 2026-10-08 13:26), built from the source tarball with default options:

```
5
7
(empty line: one row, NULL)
```

Expected `7` for the first query (the first SELECT's `WHERE (SELECT 0)` is false, and `t1 RIGHT JOIN t0` gives one row), and no row for the second (`abs(NULL)` is NULL, so the WHERE rejects the second SELECT's NULL-extended row). The same SELECTs give the right answers:

- at the top level, without the subquery: `SELECT 5 FROM t0 WHERE (SELECT 0) UNION ALL SELECT 7 FROM t1 RIGHT JOIN t0 ON 0` gives `7`;
- with the RIGHT JOIN in the first SELECT instead of the second;
- with LEFT JOIN in place of RIGHT JOIN (the second query);
- as a MATERIALIZED CTE (`WITH x AS MATERIALIZED (...) SELECT * FROM x`); NOT MATERIALIZED is wrong too;
- with the query flattener off (`.testctrl optimizations 0x1`).

DuckDB gives `7` and no row. FULL JOIN in place of RIGHT JOIN is wrong in the same way. In the first query's plan (`EXPLAIN QUERY PLAN`), the left-most SELECT is a bare `SCAN t0`, with no sign of the scalar subquery of its WHERE.

So it looks like the flattener, when it merges a UNION ALL subquery into the outer query (which `flattenSubquery` allows when the RIGHT JOIN is in a SELECT after the first and the subquery is the outer query's first FROM term), drops or misplaces WHERE terms: the first SELECT's constant term in the first query, the outer WHERE pushed into the RIGHT JOIN's SELECT in the second.

A related case was fixed in 2022: in "Unexpected Result by Union" (user forum, 2022-07-13), a UNION ALL in a subquery hid a RIGHT JOIN from the flattener, fixed by check-in 274e244c85935084. This one still reproduces on trunk, so it seems to be another path to the same kind of problem.

**Versions** (official amalgamations or tarballs, default options): 3.39.0 (the first with RIGHT JOIN), 3.42.0, 3.45.0, 3.53.4 and trunk eaec2f7f3744 and 07095b76c1f0 all give the same wrong results.

We found this with a property-based test generator for SQLite databases and queries: a common table expression of that shape gave different rows inlined, MATERIALIZED and NOT MATERIALIZED, and under TLP partitions of the outer WHERE.
