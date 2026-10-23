**Title:** SanityCheckPlan error for two window functions with `target_partitions = 1` (`WindowAggExec` loses `can_repartition` when its children are replaced)

### Describe the bug

With `datafusion.execution.target_partitions = 1`, a query with two window functions over an input with more than one partition fails to plan with a `SanityCheckPlan` distribution error. The same query runs with `target_partitions = 4`, and it runs with only one of the two window functions.

`target_partitions` is a parallelism setting, so it should not decide whether a valid query plans.

### To Reproduce

`datafusion-cli`:

```sql
SET datafusion.execution.target_partitions = 1;
SELECT count(*) OVER (PARTITION BY a + NULL) AS x,
       count(*) OVER (PARTITION BY a) AS y
FROM (SELECT 1 AS a UNION ALL SELECT 2);
```

```
SanityCheckPlan
caused by
Error during planning: Plan: ["WindowAggExec: wdw=[count(Int64(1)) PARTITION BY [a + NULL] ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING: ...]", "  UnionExec", "    ProjectionExec: expr=[1 as a]", "      PlaceholderRowExec", "    ProjectionExec: expr=[2 as a]", "      PlaceholderRowExec"] does not satisfy distribution requirements: KeyPartitioned[[NULL]]). Child-0 output partitioning: UnknownPartitioning(2)
```

(The window description is shortened.) `PARTITION BY a % nullif(0, 0)` fails the same way.

### Expected behavior

Two rows, as with `target_partitions = 4` and in DuckDB 1.5.6:

```
x,y
2,1
2,1
```

### Additional context

- `main` at c0ec5127ac99 (2026-10-22): fails as above. Release 55.2.0 fails with the same error.
- Only `target_partitions = 1` is needed; the `UNION ALL` gives the input two partitions.
- The physical planner builds `WindowAggExec` with `can_repartition = false` when `target_partitions == 1`, so the window requires `SinglePartition` and gets a `CoalescePartitionsExec`. `WindowAggExec::replace_children` with `ChildrenPropertiesMode::Recompute` (used by `with_new_children`, `datafusion/physical-plan/src/windows/window_agg_exec.rs`) rebuilds the node with `can_repartition = true` instead of `self.can_repartition`. After sort enforcement replaces its children, the window requires `KeyPartitioned`, the coalesce is removed, and the sanity check fails. `BoundedWindowAggExec` keeps `self.can_repartition` at the same place.
- Related, not the same: #21826 (`InterleaveExec::with_new_children` with changed child partitioning).
- I have a patch with tests and can open a PR.

Found with an automated differential/metamorphic testing tool; reproduced on current `main`.
