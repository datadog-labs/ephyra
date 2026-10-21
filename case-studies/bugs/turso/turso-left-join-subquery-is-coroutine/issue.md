DRAFT, not filed
Title: Comment on #8322: an ON clause with IS (or any non-indexable term) also drops materialization

Another trigger for this issue, without any affinity mismatch: when the LEFT JOIN's ON term can't drive an index seek (for example `IS`, or `coalesce(s.m, 0) = x.a`), the uncorrelated FROM-subquery is run as a coroutine once per outer row. With `=` the same query is fast.

```sql
CREATE TABLE t(a);
WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i < 600) INSERT INTO t SELECT i FROM c;
SELECT count(*) FROM t AS x LEFT JOIN (SELECT max(b.a) AS m FROM t AS b, t AS c) AS s ON s.m IS x.a;
```

Both return `600`. Turso takes 2.1 s, SQLite 3.50.4 takes 0.01 s. At 300 rows Turso takes 0.28 s, so the time grows with the cube of the row count here. `ON s.m = x.a` or `WITH s AS MATERIALIZED (...)` takes 0.01 s in Turso.

`EXPLAIN QUERY PLAN` shows `SCAN s LEFT-JOIN` with no MATERIALIZE, and `EXPLAIN` has `InitCoroutine` inside the loop over `x`.

The query that found this was a NOT MATERIALIZED CTE with a FULL JOIN over a 151-row and an 82-row table, LEFT JOINed `ON a2.c0 IS a0.c0` after a 151 x 82 cross join. It ran for more than 60 s in Turso and 0.02 s in SQLite.

Tested on Turso main abf885b2.
