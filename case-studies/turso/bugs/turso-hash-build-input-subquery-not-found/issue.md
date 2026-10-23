Filed as #10389 (https://github.com/tursodatabase/turso/issues/10389) on 2026-10-21.
Title: "Parse error: subquery not found" for a WHERE subquery that reads a table outside a materialized hash build input

```sql
CREATE TABLE t0(a);
CREATE TABLE t1(a, b);
CREATE TABLE t3(a);
INSERT INTO t0 VALUES (1), (2);
INSERT INTO t1 VALUES (1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6);
INSERT INTO t3 VALUES (1);
ANALYZE;
SELECT count(*) FROM t0 AS a0 CROSS JOIN t1 AS a1 CROSS JOIN t3 AS a2 CROSS JOIN t0 AS a3
WHERE a2.a = a1.b AND a1.a = a0.a AND (SELECT a3.a) > 0;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
  × Parse error: subquery not found
```

SQLite 3.50.4 (trunk gives the same):
```
2
```

Expected: `2`. Each of these makes Turso return 2: no `ANALYZE` (no hash join is planned), `a3.a > 0` without the subquery, or a subquery that reads `a0` or `a2` instead of `a3`. Plain `JOIN` instead of `CROSS JOIN` also works.

`EXPLAIN QUERY PLAN` for the same query with `a3.a > 0`:
```
1|0|0|MATERIALIZE hash build input for t1 AS a1
3|1|0|HASH JOIN t1 AS a1
4|1|0|SCAN t0 AS a0
51|0|0|HASH JOIN t3 AS a2
52|0|0|SCAN t0 AS a3
53|0|0|SCAN t0 AS a0
```

The subquery reads `a3`, which is outside the materialized build input. The error comes from one of the `subquery not found` checks in core/translate/planner.rs (`table_mask_from_expr` and its neighbours). They look the `SubqueryResult` id up in a subquery list, and the list they get while the build input is planned does not contain this subquery. That is an internal error, but it is reported as a parse error.

Possibly related: #8226 (hash-join plans and correlated subqueries, closed 2026-10-21) and #9648 (closed, a panic with a correlated WHERE subquery and a hash build input).

Tested on Turso main db441331.
