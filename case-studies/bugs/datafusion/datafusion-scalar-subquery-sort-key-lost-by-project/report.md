**Title:** Regression: `ORDER BY col, (scalar subquery) LIMIT` over a `RIGHT JOIN` fails in `SanityCheckPlan`

### Describe the bug

A query that orders by a column and an uncorrelated scalar subquery, with a `LIMIT`, over a `RIGHT JOIN` fails to plan
with `SanityCheckPlan ... does not satisfy order requirements`. It needs `target_partitions > 1` and a small
`batch_size`. The query works on 55.2.0 and fails on current `main`.

### To Reproduce

`datafusion-cli`:

```sql
SET datafusion.execution.target_partitions = 4;
SET datafusion.execution.batch_size = 1;
CREATE TABLE t (a INT) AS VALUES (1), (2);
SELECT t.a, (SELECT max(a) FROM t) AS m
FROM t AS u RIGHT JOIN t ON u.a = t.a
ORDER BY t.a, m LIMIT 5;
```

```
SanityCheckPlan
caused by
Error during planning: Plan: ["SortPreservingMergeExec: [a@0 ASC NULLS LAST, m@1 ASC NULLS LAST], fetch=5",
 "  ProjectionExec: expr=[a@0 as a, scalar_subquery(<pending>) as m]",
 "    HashJoinExec: mode=CollectLeft, join_type=Right, on=[(a@0, a@0)], projection=[a@1], fetch=5",
 "      DataSourceExec: partitions=1, partition_sizes=[2]",
 "      RepartitionExec: partitioning=RoundRobinBatch(4), input_partitions=1, maintains_sort_order=true",
 "        SortExec: TopK(fetch=5), expr=[a@0 ASC NULLS LAST, scalar_subquery(<pending>) ASC NULLS LAST], preserve_partitioning=[false]",
 "          DataSourceExec: partitions=1, partition_sizes=[2]"] does not satisfy order requirements: [a@0 ASC NULLS LAST, m@1 ASC NULLS LAST]. Child-0 order: [[a@0 ASC NULLS LAST]]
```

(line breaks added in the plan)

### Expected behavior

```
a,m
1,2
2,2
```

This is what 55.2.0 returns, what `main` returns without the `LIMIT` or with the default `batch_size`, and what
DuckDB 1.5.6 returns.

### Additional context

Tested on `main` at 6c1727fcd (2026-10-21, fails) and on the released `datafusion-cli` 55.2.0 (works). `git bisect`
between the 55.2.0 branch point and `main` points to ec97d1c90, "Push down topk through join" (#21621), which moves
the TopK below the join here. #21621 is not on `branch-55`, so 55.x is not affected, but it is listed for 56.0.0
(#24461), which would ship with this regression.

That change seems to expose an older gap: the projection embedded into the `HashJoinExec` keeps only columns, so the
join's output ordering `[a, scalar_subquery]` is cut to `[a]`, while the requirement above still has `m`. An
uncorrelated scalar subquery has one value for the whole query, so that sort key could be treated as constant and
dropped. `ScalarSubqueryExpr` already reports `SortProperties::Singleton`, but `EquivalenceGroup::is_expr_constant`
(`datafusion/physical-expr/src/equivalence/class.rs`) does not treat it as constant. #25926 asks for this as a
performance improvement; here it is needed for correctness. The open PR #26459 does not fix this query (I applied it
on `main` and the error is the same).

Workaround: `SET datafusion.optimizer.enable_physical_uncorrelated_scalar_subquery = false;`.

Found with an automated differential/metamorphic testing tool; reproduced on current `main`.

I have a patch with tests and can open a PR.
