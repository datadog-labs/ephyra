**Title:** `push_down_filter` substitutes the wrong expression when `col` and `CAST(col AS ...)` share a schema name (HAVING and correlated IN give type errors)

### Describe the bug

`CAST(t.a AS ...)` has the same schema name as `t.a` (see #26221). `push_down_filter` rewrites a predicate it moves below a node by looking up the node's outputs by name, in a `HashMap`. When the node has both `t.a` and `CAST(t.a AS ...)`, one entry overwrites the other, and the pushed predicate refers to the wrong one. The result is an execution-time type error for a valid query. Both branches of the rule are affected:

- `Aggregate`: `GROUP BY a, CAST(a AS VARCHAR) HAVING a > (subquery)` pushes `CAST(t.a AS Utf8View) > (subquery)` below the aggregate.
- `Projection` (`rewrite_projection`): the projection that decorrelation builds for a correlated `IN (SELECT CAST(t0.c0 AS VARCHAR) ... WHERE t0.c0 > t1.c0)` has both outputs; the filter on the CAST output is pushed as `t0.c0 = '1'`, without the CAST.

The `Aggregate` case fails on `main` with default settings (since PR #26356 fixed the `eliminate_group_by_constant` collision that used to come first). The `Projection` case is still hidden by another open name-collision bug in an earlier rule (`decorrelate_predicate_subquery`, #26033), so it shows up once that is fixed.

### To Reproduce

`datafusion-cli`, default settings, on `main`:

```sql
CREATE TABLE t (a INT) AS VALUES (1), (2);
SELECT a FROM t GROUP BY a, CAST(a AS VARCHAR) HAVING a > (SELECT min(a) FROM t);
```

```
Arrow error: Invalid argument error: Invalid comparison operation: Utf8View > Int32
```

`EXPLAIN` shows the pushed filter:

```
Projection: t.a
  Aggregate: groupBy=[[t.a, CAST(t.a AS Utf8View)]], aggr=[[]]
    Filter: CAST(t.a AS Utf8View) > (<subquery>)
```

Projection branch, with a fix for #26033 that builds the decorrelated projection with `Projection::try_new` (so this one is not reachable on `main` yet):

```sql
CREATE TABLE t0 (c0 BIGINT) AS VALUES (1);
CREATE TABLE t1 (c0 BIGINT) AS VALUES (0);
SELECT c0 FROM t1 WHERE '1' IN (SELECT CAST(t0.c0 AS VARCHAR) FROM t0 WHERE t0.c0 > t1.c0);
```

```
Arrow error: Invalid argument error: Invalid comparison operation: Int64 == Utf8View
```

### Expected behavior

First query: one row, `a = 2`. Second query: one row, `c0 = 0`. DuckDB 1.5.6 returns these rows.

### Additional context

- `main` at c0ec5127ac99 (2026-10-22): the first query fails as above on default settings. Release 55.2.0 first fails in `eliminate_group_by_constant` (fixed on `main` by #26356); with `SET datafusion.optimizer.skip_failed_rules = true` it gives the same type error. The second query fails with the #26033 error on `main` and 55.2.0, and with the type error once #26033 is fixed as described.
- With a literal (`HAVING a > 1`) the first query is correct; the scalar subquery is needed.
- Where in the code (`datafusion/optimizer/src/push_down_filter.rs`): in the `LogicalPlan::Aggregate` branch, `replace_map` is keyed by `expr.schema_name()` for each group expression; in `rewrite_projection`, the map is keyed by `qualified_name(qualifier, field)`. Two outputs with the same name collide in both.
- Related: #26221 (CAST keeps its input's name; aggregate schema), #26033 (open) and PR #26356 (merged): the same collision in other rules.
- I have a patch with tests and can open a PR (predicates on names that occur twice stay above the node).

Found with an automated differential/metamorphic testing tool; reproduced on current `main`.
