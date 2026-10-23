Filed as #9848 (https://github.com/tursodatabase/turso/issues/9848) (line 1 corrected 2026-10-21).

# "Parse error: all before-loop subqueries should have already been emitted" for a materialized CTE with a correlated subquery, read from a nested subquery

## Reproduction

```sql
CREATE TABLE t0(c0);
INSERT INTO t0 VALUES(7);
WITH x AS MATERIALIZED (SELECT (SELECT c0) AS c0 FROM t0)
SELECT * FROM (SELECT (SELECT c0 FROM x));
```

The same error appears without `MATERIALIZED` when the CTE is read twice, which makes Turso materialize it:

```sql
WITH x AS (SELECT (SELECT c0) AS c0 FROM t0)
SELECT max(1) OVER (), (SELECT count(*) FROM x AS a, x AS b);
```

## Turso output

```
  × Parse error: all before-loop subqueries should have already been emitted,
  │ but found 1 unevaluated subqueries
```

## SQLite 3.50.4 output

```
7
1|1
```

(Trunk gives the same.)

## Expected

The SQLite results. The error is an internal check reported as a "Parse error", for a valid query.

## What is needed to trigger it

- The CTE is materialized (`MATERIALIZED`, or read more than once).
- The CTE body has a subquery that reads the CTE's own FROM table (`(SELECT c0)`, `t0.c0 IN (SELECT c0)`, `WHERE EXISTS (SELECT t0.c0)`).
- The CTE is read inside a subquery of a query that is itself nested: a derived table `SELECT * FROM (SELECT (SELECT ... FROM x))`, or a query with a window function (which Turso plans as a nested query).

Each of these works: the same query without the outer derived table (`SELECT (SELECT c0 FROM x)`), an uncorrelated subquery in the CTE body (`(SELECT 1)`), a view or a plain derived table instead of the CTE.

## Where

The error comes from the check at the end of the loop-open code in core/translate/main_loop/open.rs (around line 307). I did not find the root cause. The subquery inside the CTE body has eval phase `BeforeLoop`, but in this nesting `emit_non_from_clause_subqueries_for_phase` (core/translate/subquery.rs) never emits it, because its computed `get_eval_at` does not match the phase being emitted.

Tested on Turso main at 7641fad2 (2026-10-13).
