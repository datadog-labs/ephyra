**Title:** SanityCheckPlan error for `UNION ALL ... ORDER BY` when each arm selects the same column twice

### Describe the bug

Each arm of a `UNION ALL` outputs the same column under two names (`a AS x, a AS y`), and the query orders by both names. Planning then fails with a `SanityCheckPlan` error. Default settings are enough to hit it.

EnforceSorting sorts each union input on `a` only. That is correct, because `x = y` in each input. But the `UnionExec` output ordering is built from each input's normalized ordering. That ordering leaves out `y`, because `y` is equal to an earlier key. So the union reports only `[x]` as its ordering, and the `SortPreservingMergeExec` requirement `[x, y]` is rejected.

### To Reproduce

```sql
CREATE TABLE t(a INT) AS VALUES (2), (1);
SELECT a AS x, a AS y FROM t UNION ALL SELECT a AS x, a AS y FROM t ORDER BY x, y;
```

```
# datafusion-cli (main):
SanityCheckPlan
caused by
Error during planning: Plan: ["SortPreservingMergeExec: [x@0 ASC NULLS LAST, y@1 ASC NULLS LAST]", "  UnionExec", "    ProjectionExec: expr=[a@0 as x, a@0 as y]", "      SortExec: expr=[a@0 ASC NULLS LAST], preserve_partitioning=[false]", "        DataSourceExec: partitions=1, partition_sizes=[1]", "    ProjectionExec: expr=[a@0 as x, a@0 as y]", "      SortExec: expr=[a@0 ASC NULLS LAST], preserve_partitioning=[false]", "        DataSourceExec: partitions=1, partition_sizes=[1]"] does not satisfy order requirements: [x@0 ASC NULLS LAST, y@1 ASC NULLS LAST]. Child-0 order: [[x@0 ASC NULLS LAST]]
```

### Expected behavior

The query is valid, so it should return the four rows in order:

```
# sqlite3: 1|1, 1|1, 2|2, 2|2
# datafusion-cli: SanityCheckPlan error (above)
```

The `UnionExec` properties are documented as what holds for every input: "Since the output equivalence properties are properties that are true for *all* output partitions, that is the same as being true for all *input* partitions." (`calculate_union_binary` in `union.rs`). Here `[x, y]` holds for both inputs, but it is missing from the union's output ordering.

### Additional context

- Tested on `main` at c0ec5127ac99 (2026-10-22): fails as shown. The 55.2.0 release (`datafusion-cli`) fails the same way.
- It also fails with `SET datafusion.execution.target_partitions = 1;`. An empty table fails too.
- Related, but not the same bug: #17372 (Sanity Check fails on UnionExec + constants, closed), #26391, #26454.
- Where in the code: `UnionEquivalentOrderingBuilder::add_satisfied_orderings` in `datafusion/physical-expr/src/equivalence/properties/union.rs` only tries `source.oeq_cache.normal_cls`. Normalized orderings drop sort keys that are equal to an earlier key, and the input equivalences are not kept at the union output (see the `TODO` in `calculate_union`). So orderings like `[x, y]` are lost.

I have a patch with tests and can open a PR.

Found with an automated differential/metamorphic testing tool; reproduced on current `main`.
