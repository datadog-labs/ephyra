**Title:** Wrong results: `ORDER BY <double>, ... LIMIT n` drops `-0.0` rows when 0.0 arrives in an earlier batch (TopK dynamic filter)

### Describe the bug

`ORDER BY x, y LIMIT 2` can leave out a row that belongs in the top 2 when `x` is a `DOUBLE` column holding both
`0.0` and `-0.0`. It happens when the rows come in more than one batch. Default settings.

The TopK heap orders `-0.0` before `0.0`, the same way the full sort does. After the first batch, TopK builds a
dynamic filter such as `x < 0.0 OR (x = 0.0 AND y < 1)`. Since #22835, comparisons treat `-0.0 = 0.0`, so the
filter throws away `-0.0` rows that the heap would have kept.

### To Reproduce

Run in `datafusion-cli`. The two `INSERT`s give the table two batches.

```sql
CREATE TABLE t (x DOUBLE, y INT);
INSERT INTO t VALUES (0.0, 1), (-0.0, 7);
INSERT INTO t VALUES (-0.0, 5);
SELECT x, y FROM t ORDER BY x, y;
SELECT x, y FROM t ORDER BY x, y LIMIT 2;
```

Actual output:

```
+------+---+
| x    | y |
+------+---+
| -0.0 | 5 |
| -0.0 | 7 |
| 0.0  | 1 |
+------+---+
+------+---+
| x    | y |
+------+---+
| -0.0 | 7 |
| 0.0  | 1 |
+------+---+
```

### Expected behavior

The `LIMIT 2` result should be the first two rows of the full `ORDER BY` result:

```
SELECT x, y FROM t ORDER BY x, y LIMIT 2;
# DuckDB 1.5.6:  (0.0, 1), (0.0, 5)   -- DuckDB stores -0.0 as 0.0
# DataFusion:    (-0.0, 7), (0.0, 1)
# expected:      (-0.0, 5), (-0.0, 7)
```

Under any ordering, `(-0.0, 5)` comes before `(-0.0, 7)`: the `x` values are identical and `5 < 7`. So the result
cannot contain `(-0.0, 7)` and leave out `(-0.0, 5)`. If all three rows are inserted in one `INSERT` (one batch),
the query returns `(-0.0, 5), (-0.0, 7)`.

A single-key query shows the same thing. After `INSERT INTO t VALUES (0.0, 1); INSERT INTO t VALUES (-0.0, 2);`,
`SELECT x FROM t ORDER BY x LIMIT 1` returns `0.0`, but `SELECT x FROM t ORDER BY x` lists `-0.0` first.

### Additional context

- Tested on `main` at c0ec5127ac99 (2026-10-22) and on release 55.2.0. Both return the wrong result above.
- Where in the code: `TopK::build_filter_expression` (`datafusion/physical-plan/src/topk/mod.rs`) builds `Lt`/`Gt`/`Eq`
  on the threshold. Those comparisons normalize `-0.0` to `0.0` (`apply_cmp` in
  `datafusion/physical-expr-common/src/datum.rs`, added in #22835), but the TopK heap compares rows with the
  row format, which puts `-0.0` before `0.0`. The filter must never drop a row that the heap would keep.
- Related: #26138 (epic: handle `-0.0` float keys in one place), #26491 (same kind of mismatch in Parquet bloom
  filter pruning), #22490 / #22826 / #22835 (the comparison change).

Found with an automated differential/metamorphic testing tool; reproduced on current `main`.
