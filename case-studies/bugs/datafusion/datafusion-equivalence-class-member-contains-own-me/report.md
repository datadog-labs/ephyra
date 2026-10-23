**Title:** SanityCheckPlan error with sort-merge join when a filter says `a + b = a`

### Describe the bug

If a filter contains an equality between an expression and one of its own sub-expressions, such as
`x.a + x.b = x.a`, then a sort-merge join on `(a, b)` above that filter fails physical planning with a
`SanityCheckPlan` error. The same query works with the default `prefer_hash_join = true`.

### To Reproduce

Run in `datafusion-cli`:

```sql
CREATE TABLE t (a INT, b INT) AS VALUES (1, 0), (2, 3);
SET datafusion.optimizer.prefer_hash_join = false;
SELECT * FROM t x JOIN t y ON x.a = y.a AND x.b = y.b WHERE x.a + x.b = x.a;
```

Actual output on `main`:

```
SanityCheckPlan
caused by
Error during planning: Plan: ["SortMergeJoinExec: join_type=Inner, on=[(a@0, a@0), (b@1, b@1)]", "  SortExec: expr=[a@0 + b@1 ASC, b@1 ASC], preserve_partitioning=[true]", "    RepartitionExec: partitioning=Hash([a@0, b@1], 16), input_partitions=1", "      FilterExec: a@0 + b@1 = a@0", "        DataSourceExec: partitions=1, partition_sizes=[1]", "  SortExec: expr=[a@0 + b@1 ASC, b@1 ASC], preserve_partitioning=[true]", "    RepartitionExec: partitioning=Hash([a@0, b@1], 16), input_partitions=1", "      FilterExec: a@0 + b@1 = a@0", "        DataSourceExec: partitions=1, partition_sizes=[1]"] does not satisfy order requirements: [a@0 ASC, b@1 ASC]. Child-0 order: [[a@0 + b@1 ASC, b@1 ASC]]
```

### Expected behavior

The query should return the one matching row, as it does with the default settings:

```
SELECT * FROM t x JOIN t y ON x.a = y.a AND x.b = y.b WHERE x.a + x.b = x.a;
# SQLite 3 / DataFusion with prefer_hash_join = true: 1 | 0 | 1 | 0
# DataFusion with prefer_hash_join = false: SanityCheckPlan error (above)
```

`prefer_hash_join` is documented as a preference: "When set to true, the physical plan optimizer will prefer
HashJoin over SortMergeJoin." It should not decide whether a valid query plans at all.

### Additional context

- Tested on `main` at c0ec5127ac99 (2026-10-22): fails. Latest release 55.2.0: fails the same way.
- With a single join key (`ON x.a = y.a`) the query works.
- Where in the code: `FilterExec` turns `a + b = a` into the equivalence class `{a + b, a}` with `a + b` as the
  canonical member. `EquivalenceGroup::normalize_expr` (`datafusion/physical-expr/src/equivalence/class.rs`)
  rewrites bottom-up, so it also rewrites the `a` inside `a + b` and gets `(a + b) + b`, which is not in the class.
  The sort that `EnforceSorting` inserts for the join keys is then not recognized as satisfying `[a, b]`.
- Related: #20194, fixed by #20231, was the same normalization problem with a literal (`expr = literal`). That fix
  skips equalities where neither side is a column. `a + b = a` has a column on one side, so it is still affected.
- I have a patch with tests and can open a PR. It makes `EquivalenceGroup::add_equal_conditions` skip an equality
  that would put an expression in the same class as one of its own sub-expressions. That only gives up an
  optimization.
- Found with an automated differential/metamorphic testing tool; reproduced on current `main`.
