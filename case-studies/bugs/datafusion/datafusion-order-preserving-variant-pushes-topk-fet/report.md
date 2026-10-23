**Title:** Wrong result: `count(*) OVER ()` sees only the LIMIT rows when ORDER BY ... LIMIT sits on top of another window (target_partitions > 1)

### Describe the bug

With more than one target partition, the LIMIT of an outer `ORDER BY ... LIMIT` gets pushed below a `WindowAggExec` that needs every input row. Here `count(*) OVER ()` should count all 3 rows. It returns 1, because the window only sees the 1 row the LIMIT keeps.

With `target_partitions = 1` the same query returns the right answer. On a multi-core machine the default settings also show the bug (the `SET` below just makes it independent of the machine).

### To Reproduce

```sql
SET datafusion.execution.target_partitions = 4;
SELECT count(*) OVER () AS n,
       row_number() OVER (PARTITION BY length('x') ORDER BY a) AS r
FROM (SELECT 1 AS a UNION ALL SELECT 2 UNION ALL SELECT 3)
ORDER BY r LIMIT 1;
```

```
# DuckDB 1.5.6:                  n=3, r=1
# sqlite3:                       n=3, r=1
# DataFusion target_partitions=1: n=3, r=1
# DataFusion target_partitions=4: n=1, r=1
+---+---+
| n | r |
+---+---+
| 1 | 1 |
+---+---+
```

The physical plan (`EXPLAIN`, indent format, shortened) shows the `fetch=1` under the window:

```
ProjectionExec: expr=[count(...) ... as n, row_number() ... as r]
  WindowAggExec: wdw=[count(Int64(1)) ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING ...]
    SortPreservingMergeExec: [row_number() ... ASC NULLS LAST], fetch=1
      ProjectionExec: ...
        BoundedWindowAggExec: wdw=[row_number() PARTITION BY [length(Utf8("x"))] ORDER BY [a ASC NULLS LAST] ...], mode=[Sorted]
          SortExec: expr=[a@0 ASC NULLS LAST], preserve_partitioning=[true]
            RepartitionExec: partitioning=Hash([1], 4), input_partitions=3
              UnionExec
```

There is no `SortExec`/TopK above the `WindowAggExec`; the limit is applied only inside the `SortPreservingMergeExec` below it.

### Expected behavior

```
+---+---+
| n | r |
+---+---+
| 3 | 1 |
+---+---+
```

`count(*) OVER ()` is computed over all rows of its input; `ORDER BY r LIMIT 1` should only drop rows after the window is computed. `target_partitions` is a performance setting ("Number of partitions for query execution. Increasing partitions can increase concurrency.", `datafusion/common/src/config.rs`), so it should not change the result. Its default is the number of CPU cores, so the bug also shows with default settings.

### Additional context

- Reproduces on `main` at c0ec5127ac99 (2026-10-22) and on the 55.2.0 release (`datafusion-cli`).
- Where in the code: in `EnsureRequirements`, `plan_with_order_preserving_variants` (`datafusion/physical-optimizer/src/ensure_requirements/enforce_sorting/replace_with_order_preserving_variants.rs`) replaces the `CoalescePartitionsExec` under the `WindowAggExec` with a `SortPreservingMergeExec` and gives it the outer sort's `fetch` (`.with_fetch(fetch)`), even though the `WindowAggExec` between them needs all rows. The outer TopK sort is then removed.
- Possibly related, but they look different: #26162 (SPM with fetch, wrong ordering), #21169 and #24807 (fetch lost, the opposite direction), #25066 (PR, "preserve window inputs during TopK aggregation").

Found with an automated differential/metamorphic testing tool; reproduced on current `main`.
