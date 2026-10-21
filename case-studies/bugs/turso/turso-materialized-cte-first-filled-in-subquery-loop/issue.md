DRAFT, not filed

# Panic "cursor id N is None" when a materialized CTE is first read inside a nested subquery's loop and then read by the outer query

## Reproduction

```sql
CREATE TABLE t0(c0);
WITH x AS MATERIALIZED (SELECT 0 AS a)
SELECT a, (SELECT (SELECT a FROM x WHERE a = t0.c0) FROM t0) FROM x;
```

Run with `tursodb -q -m list test.db < repro.sql`.

## Turso output

```
thread 'main' panicked at core/vdbe/mod.rs:1499:32:
cursor id 1 is None
```

## SQLite 3.50.4 output

```
0|
```

(Trunk gives the same.)

## Expected

Same as SQLite.

## Notes

- `t0` must be empty, or the nested read of `x` must sit on a path that does not run (a correlated subquery in an empty loop, or a `HAVING` subquery of a `GROUP BY` with no groups). With a row in `t0` the query works.
- Without `MATERIALIZED` it also happens when Turso materializes the CTE because it cannot flatten it, for example `WITH x AS (SELECT 0 AS a UNION SELECT 1) SELECT x.a, (SELECT (SELECT y.a FROM x AS y WHERE y.a = t0.c0) FROM t0) FROM x;`.
- Other forms: `WITH x AS MATERIALIZED (SELECT 0) SELECT (SELECT 1 FROM t0 GROUP BY c0 HAVING EXISTS (SELECT * FROM x)) FROM x;` and `... SELECT 1 FROM x WHERE EXISTS (SELECT 1 FROM t0 GROUP BY c0 HAVING EXISTS (SELECT * FROM x));`.

## What happens

EXPLAIN shows the order. The select-list subquery is uncorrelated, so `emit_query` emits it before the FROM clause. Inside it, the correlated subquery over `x` is the first reference to `x`, so the code that fills `x` (`Once`, `OpenEphemeral 1`, `Insert`) goes into the loop over `t0`. The outer `FROM x` then finds `x` already registered and opens it with `OpenDup 3 1`. When `t0` is empty, `OpenEphemeral 1` never runs, and `OpenDup` reads an unopened cursor.

`emit_from_clause_subqueries` (core/translate/subquery.rs) pre-materializes multi-reference CTEs first ("otherwise a coroutine could try to OpenDup a CTE whose backing table has not been created yet"), but in `emit_query` (core/translate/emitter/select.rs) the before-loop subqueries are emitted before that, so a CTE can get its first, conditional fill from inside one of them.

Related but a different symptom: #9848 (materialized CTE, before-loop subquery not emitted).

Tested on Turso main at f8549ca2 (2026-10-13).
