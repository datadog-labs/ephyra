**Title:** `= ANY (subquery)` in a `JOIN ... ON` over an aliased table fails: "No field named x.a"

### Describe the bug

A quantified comparison (`= ANY`, `> ALL`) with a subquery in the `ON` clause of a join fails to plan when the column
it compares comes from an aliased table. The same predicate works in `WHERE`, without the alias, or written as `IN`.

### To Reproduce

`datafusion-cli`:

```sql
CREATE TABLE t (a INT) AS VALUES (1), (2);
CREATE TABLE s (b INT) AS VALUES (1);
SELECT * FROM t AS x JOIN t AS y ON x.a = ANY (SELECT b FROM s);
```

```
Optimizer rule 'simplify_expressions' failed
caused by
Schema error: No field named x.a. Did you mean 't.a'?
Valid fields are t.a, __correlated_sq_1.mark, __correlated_sq_2.b.
```

These equivalent queries work and return the expected rows:

```sql
SELECT * FROM t AS x, t AS y WHERE x.a = ANY (SELECT b FROM s);
SELECT * FROM t AS x JOIN t AS y ON x.a IN (SELECT b FROM s);
```

### Expected behavior

```
a,a
1,1
1,2
```

`x.a = ANY (SELECT b FROM s)` holds only for `x.a = 1`, which joins with both rows of `y`. DuckDB 1.5.6 returns the
same rows.

### Additional context

Tested on `main` at 6c1727fcd (2026-10-21) and on the released `datafusion-cli` 55.2.0: both fail (55.2.0 lists
slightly different valid fields).

From `EXPLAIN VERBOSE` with `datafusion.optimizer.skip_failed_rules = true`: `rewrite_set_comparison` turns the
`= ANY` into `EXISTS` subqueries that reference `outer_ref(x.a)`. The ON predicate becomes a filter, and
`push_down_filter` pushes it through `SubqueryAlias x` (the `LogicalPlan::SubqueryAlias` arm in
`datafusion/optimizer/src/push_down_filter.rs`). `replace_cols_by_name` there rewrites `Expr::Column` only, so the
outer reference inside the subquery keeps the qualifier `x`, which does not exist below the alias. A later
schema-checked rule then fails. The nearest existing reports I found are #25258 and PR #26044, which are about
different cases.

Found with an automated differential/metamorphic testing tool; reproduced on current `main`.

I have a patch with tests and can open a PR.
