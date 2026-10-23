**Title:** SanityCheckPlan error with sort-merge join when a filter makes a sort or join key constant (constant lost in `EquivalenceGroup` projection)

### Describe the bug

With `datafusion.optimizer.prefer_hash_join = false`, queries where a `WHERE` clause makes an expression constant, and that expression is then needed as a sort or join key above a `SortMergeJoinExec`, fail to plan with `SanityCheckPlan`. `EnforceSorting` drops the sort on the constant key. Then `ProjectionPushdown` embeds a projection into the `SortMergeJoinExec`, and when the equivalence properties are projected through it the constant is lost, so the requirement above is no longer met.

The same queries work with the default hash join, in 55.2.0, and in DuckDB 1.5.6. `prefer_hash_join` is a planner preference, so it should not decide whether a valid query plans.

### To Reproduce

`datafusion-cli`, first query (constant `t1.b + 1` used as a sort key):

```sql
SET datafusion.optimizer.prefer_hash_join = false;
CREATE TABLE t (a INT, b INT) AS VALUES (1, 1);
SELECT t1.b + 1 AS x, t2.a FROM t t1 JOIN t t2 ON t1.a = t2.a WHERE t1.b + 1 = 2 ORDER BY x, t2.a;
```

```
SanityCheckPlan
caused by
Error during planning: Plan: ["SortPreservingMergeExec: [x@0 ASC NULLS LAST, a@1 ASC NULLS LAST]", "  SortExec: expr=[a@1 ASC NULLS LAST], preserve_partitioning=[true]", "    ProjectionExec: expr=[CAST(b@0 AS Int64) + 1 as x, a@1 as a]", "      SortMergeJoinExec: join_type=Inner, on=[(a@0, a@0)], projection=[b@1, a@2]", "        SortExec: expr=[a@0 ASC], preserve_partitioning=[true]", "          RepartitionExec: partitioning=Hash([a@0], 16), input_partitions=1", "            FilterExec: CAST(b@1 AS Int64) + 1 = 2", "              DataSourceExec: partitions=1, partition_sizes=[1]", "        SortExec: expr=[a@0 ASC], preserve_partitioning=[true]", "          RepartitionExec: partitioning=Hash([a@0], 16), input_partitions=1", "            DataSourceExec: partitions=1, partition_sizes=[1]"] does not satisfy order requirements: [x@0 ASC NULLS LAST, a@1 ASC NULLS LAST]. Child-0 order: [[a@1 ASC NULLS LAST]]
```

Second query (constant `t1.b + (scalar subquery)` used as a join key of a second sort-merge join):

```sql
SET datafusion.optimizer.prefer_hash_join = false;
CREATE TABLE t (a INT, b INT) AS VALUES (1, 1), (2, 0);
SELECT * FROM (SELECT t1.b + (SELECT max(a) FROM t) AS x FROM t t1 JOIN t t2 ON t1.a = t2.a) s JOIN t t3 ON s.x = t3.a WHERE s.x = 2;
```

```
SanityCheckPlan
caused by
Error during planning: Plan: ["SortMergeJoinExec: join_type=Inner, on=[(x@0, a@0)]", "  RepartitionExec: partitioning=Hash([x@0], 16), input_partitions=16", "    ProjectionExec: expr=[b@0 + scalar_subquery(<pending>) as x]", "      SortMergeJoinExec: join_type=Inner, on=[(a@0, a@0)], projection=[b@1]", ...] does not satisfy order requirements: [x@0 ASC]. Child-0 order: []
```

(Partition counts depend on the number of cores.)

### Expected behavior

First query: one row `x = 2, a = 1`. Second query: one row `x = 2, a = 2, b = 0`. DuckDB 1.5.6, DataFusion 55.2.0, and `main` without the `SET` all return these rows.

### Additional context

- `main` at c0ec5127ac99 (2026-10-22): both fail. Release 55.2.0 returns the right rows, because it cannot embed a projection into `SortMergeJoinExec` (added in #24517), but the code below is the same there.
- Non-default setting needed: `datafusion.optimizer.prefer_hash_join = false`. The first query also fails with `LEFT JOIN`, and with `upper(t1.s) = 'X'` / `ORDER BY upper(t1.s)`; a plain column (`WHERE t1.b = 1`) works.
- There are two gaps, both in `datafusion/physical-expr/src/equivalence/class.rs`: `EquivalenceGroup::project` rebuilds each class with `EquivalenceClass::new(...)` and drops the class's `constant` (first query), and `EquivalenceGroup::project_expr_indirect` treats `Literal` leaves as valid under any projection but not `ScalarSubqueryExpr` leaves (second query, which also needs the first fix).
- Related: #26283 / PR #26459 (the same dropped `constant` in `EquivalenceClass::try_with_offset` for joins), #26391 / PR #26397 (constant lost through a duplicate alias), #26572 (scalar subquery sort key, `is_expr_constant`). With #26397 and #26459 applied on `main`, both queries still fail.
- I have a patch with tests and can open a PR.

Found with an automated differential/metamorphic testing tool; reproduced on current `main`.
