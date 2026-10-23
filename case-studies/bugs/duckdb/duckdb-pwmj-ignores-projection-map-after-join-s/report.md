# Wrong results / INTERNAL Error from a piecewise merge join above an outer join with a subquery in its ON clause

> Note for the owner. DuckDB's `AI_POLICY.md` says "Autonomous agents are not allowed to be used for
> contributing to this project" and AI should not write the communication. Use this packet as working
> notes and write the issue yourself. The issue template (`.github/ISSUE_TEMPLATE/bug_report.yml`) also
> requires **Full Name** and **Affiliation**, which only you can fill in. `fix.diff` is for your reference
> only and should not be submitted as is.
>
> This shares its root cause with the packet `bugs/nlj-ignores-projection-map-after-join-su` (nested loop /
> blockwise NL join). Consider filing them as one issue, or as two linked issues.

**Trunk status:** still reproduces on current main `1b4c3e54ba` (2026-10-16, `v2.1.0-dev89158`). Not fixed upstream.
Released v1.5.6 gives the correct result for the wrong-result query, so that part is a **regression since v1.5.6**.

## What happens

Take an outer join whose `ON` clause contains an uncorrelated subquery, and join its result to another
table on an inequality (`<`) condition. If both sides have at least `nested_loop_join_threshold` (default 5)
rows, the inequality join is planned as a `PIECEWISE_MERGE_JOIN`. That operator then emits columns from the
wrong positions. Depending on the column types, the query either

- silently returns wrong rows (query 2 below), or
- fails with `INTERNAL Error: Vector::Reference used on vector of different type`, after which the database
  instance is invalidated (query 1 below).

Both queries are valid.

## To Reproduce

Query 1 (INTERNAL error):

```sql
CREATE TABLE t0(c0 INTEGER);
INSERT INTO t0 SELECT range FROM range(5);
SELECT count(*) FROM t0 AS x1 LEFT JOIN t0 AS x2 ON (1 IN (SELECT 1)) JOIN t0 AS x3 ON (x1.c0 < x3.c0);
```

Expected: `50`. The LEFT JOIN condition is always true, so it yields all 5×5 pairs. `x1.c0 < x3.c0` holds
for 10 (x1, x3) pairs, and each of those combines with all 5 `x2` rows: 10×5 = 50.

Actual (main `1b4c3e54ba`):

```
INTERNAL Error: Vector::Reference used on vector of different type (source INTEGER referenced BIGINT)
...
This error signals an assertion failure within DuckDB. This usually occurs due to unexpected conditions or errors in the program's logic.
For more information, see https://duckdb.org/docs/current/dev/internal_errors
```

v1.5.6 rejects this query with `Not implemented Error: Cannot perform non-inner join on subquery!`, so for
this query it is not a regression.

Query 2 (silent wrong result):

```sql
CREATE TABLE t0(c0 INTEGER);
INSERT INTO t0 SELECT range FROM range(5);
SELECT x1.c0, x2.c0, x3.c0 FROM t0 AS x1 RIGHT JOIN t0 AS x2 ON ((SELECT 3) = x1.c0) JOIN t0 AS x3 ON (x2.c0 < x3.c0) ORDER BY ALL;
```

Expected (v1.5.6 returns exactly this; so does main with the subquery replaced by the literal `3`):

```
3,0,1
3,0,2
3,0,3
3,0,4
3,1,2
3,1,3
3,1,4
3,2,3
3,2,4
3,3,4
```

Actual on main `1b4c3e54ba`:

```
0,3,3
0,3,3
0,3,3
0,3,3
1,3,3
1,3,3
1,3,3
2,3,3
2,3,3
3,3,3
```

The `x1` column shows `x2`'s values, and the `x2` and `x3` columns both show the subquery value `3`. The row
count is right because the join condition itself is evaluated on the correct columns; only the output is
taken from the wrong positions. `EXPLAIN` shows a `Piecewise Merge Join` (INNER) on top of a
`Blockwise Nl Join` (LEFT).

The same as a sqllogictest: `repro/test_subquery_outer_join_condition_pwmj.test`.
`repro/repro.sh <path-to-duckdb-cli>` runs both queries. It exits 1 if either bug reproduces, 0 if both give
the expected result, and 2 if it cannot decide (v1.5.6 gives 2 because it rejects query 1).

Setting `nested_loop_join_threshold` high enough that the join becomes a nested loop join makes query 1 hit
the nested-loop variant of this bug instead (assertion `op.left_projection_map.empty() || op.join_type ==
JoinType::MARK` in `plan_comparison_join.cpp`), which is the companion issue.

## Expected behaviour / contract

These are the universal rules: no wrong results and no internal errors. DuckDB's own message for this class
of error says: "This error signals an assertion failure within DuckDB. This usually occurs due to unexpected
conditions or errors in the program's logic." (https://duckdb.org/docs/current/dev/internal_errors)
For query 2, v1.5.6 returns the expected rows, and so does main for the same query with `(SELECT 3)`
replaced by `3`.

## Environment

- OS: Linux x86_64 (Ubuntu, kernel 6.8), gcc 13
- DuckDB version: `v2.1.0-dev89158 (Development Version) 1b4c3e54ba` (main, 2026-10-16), built
  RelWithDebInfo with `-DFORCE_ASSERT=1` and extensions core_functions, parquet, json, icu, tpch
- Originally found on main `ba16e6f761` (2026-10-16) with local fixes for two other bugs applied (one is the companion NLJ fix), same build flags
- v1.5.6 (`069cc9f9b5`, official `duckdb_cli-linux-amd64` release binary): query 1 is rejected as not
  implemented, query 2 is correct
- Client: CLI

`Vector::Reference` checks the type with a plain `throw InternalException`, not a `D_ASSERT`, so query 1 should
fail the same way in release builds. We only ran the assertion-enabled build of main, though.

## Root cause (analysis, from reading the code with the fix below)

1. `RecursiveDependentJoinPlanner` (`src/planner/binder/query_node/plan_subquery.cpp`) plans the uncorrelated
   subquery of the outer join's `ON` clause into a child of that join. This adds a subquery column to the
   join's output.
2. When the parent join's child is rewritten, `ColumnBindingRewrite::RewriteChild`
   (`src/optimizer/column_binding_replacer.cpp`, line 318) freezes a projection map on the parent comparison
   join so that the new column stays hidden. This behaviour came in with 35fca1ca1d "Preserve projection
   selections when rewriting logical children" (2026-09-30). That commit is not in any release tag. We think
   it made the bug reachable, but we did not bisect.
3. `PlanComparisonJoin` (`src/execution/physical_plan/plan_comparison_join.cpp`) plans the parent `<` join as
   a `PhysicalPiecewiseMergeJoin`. The comment there says "range joins apply both projection maps", but only the
   IE join does (through `PhysicalRangeJoin::ProjectResult`). `physical_piecewise_merge_join.cpp` never reads
   the projection maps. It fills the output chunk, whose types come from the projected logical layout,
   position by position from the unprojected left payload. So every column after the hidden one is shifted.
   If the types differ, `Vector::Reference` throws. If they match, wrong values come out. Earlier in
   the same function (line 240) a comment still reads `// TODO: Extend PWMJ to handle all comparisons and projection maps`.
4. Before d1654374688 (PR #27081, merged 2026-10-13), `D_ASSERT(op.left_projection_map.empty())` sat in front
   of this path, so assertion builds failed with an assertion there. That PR removed it.

## Suggested fix

Plan the piecewise merge join on the full child layouts and put the projection on top. Clear the projection
maps, set the op's types to the unprojected left+right layout, build the PWMJ, and add a `PhysicalProjection`
that selects the columns the projection maps asked for. The nested-loop fix in the companion issue does the
same thing with a helper `PhysicalPlanGenerator::PlanJoinWithoutProjectionMaps`. This fix only wraps the PWMJ
construction in that helper and corrects the comment.

`fix.diff` is written against main `1b4c3e54ba` and applies cleanly there with `git apply`. It contains the
companion nested-loop fix (the helper, `column_binding_resolver.cpp`, `plan_any_join.cpp`, and its test)
plus the PWMJ change and the test `test/sql/subquery/scalar/test_subquery_outer_join_condition_pwmj.test`.

Making `PhysicalPiecewiseMergeJoin` apply the projection maps itself, as the IE join does, would be another
approach.

Checked against that patch, on relassert builds:

- rerun for this packet: `repro/repro.sh` exits 1 on unpatched main `1b4c3e54ba` and 0 on `1b4c3e54ba` + `fix.diff`
  (query 1 returns 50, query 2 returns the expected 10 rows). Only the CLI was built, so the sqllogictest
  itself was not run through `unittest` here
- per the triage notes, not rerun for this packet: on `ba16e6f761` plus the patch, `test/sql/join/*`
  (181 files), `test/sql/subquery/*` (108) and `test/optimizer/*` (232) pass
