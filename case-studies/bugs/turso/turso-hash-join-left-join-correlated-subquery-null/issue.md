Filed as #10388 (https://github.com/tursodatabase/turso/issues/10388) on 2026-10-21.
Title: Correlated subquery in the SELECT list reads NULL for a hash-build-side table when the probe side is a LEFT JOIN (outer-join case of #8226)

```sql
CREATE TABLE t(a, b, c);
CREATE TABLE u(b);
INSERT INTO t VALUES (1, NULL, 10), (2, 1, 20);
INSERT INTO u VALUES (2);
ANALYZE;
SELECT t1.c, (SELECT t1.c), EXISTS (SELECT 1 WHERE t1.c = 10) FROM t AS t1 JOIN t AS t2 ON t2.b = t1.a LEFT JOIN u ON u.b = t2.a;
SELECT t1.c, (SELECT t1.c), EXISTS (SELECT 1 WHERE t1.c = 10) FROM t AS t1 JOIN t AS t2 ON t2.b = t1.a JOIN u ON u.b = t2.a;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
10||0
10|10|1
```

SQLite 3.50.4 (trunk gives the same):
```
10|10|1
10|10|1
```

Expected: `(SELECT t1.c)` is the row's own `t1.c`, 10, and the EXISTS is true. With the LEFT JOIN Turso returns NULL and false; the same query with an inner join (second SELECT) is right, as is the LEFT JOIN query without `ANALYZE` (no hash join is chosen then).

`EXPLAIN QUERY PLAN` for the LEFT JOIN query shows the shape #8226 described, a copy of the subquery inside the materialized build input and one outside it:
```
1|0|0|MATERIALIZE hash build input for t AS t2
3|1|0|SCAN t AS t1
4|1|0|SEARCH t2 USING COVERING INDEX ephemeral_t_t2 (b=?)
8|1|0|CORRELATED SCALAR SUBQUERY 1
11|8|0|SCAN CONSTANT ROW
49|0|0|CORRELATED SCALAR SUBQUERY 2
52|49|0|SCAN CONSTANT ROW
59|0|0|HASH JOIN u
```
In the bytecode the outer copy runs once, after the build-input loop has finished and before `u` is opened, so it reads `t1` past its last row; the result row then uses that value instead of the `t1.c` carried in the hash payload. #8226 was closed today after its inner-join reproducers (and the outer-join ones in its comments) were fixed; this LEFT JOIN, SELECT-list form still fails.

Tested on Turso main db441331.
