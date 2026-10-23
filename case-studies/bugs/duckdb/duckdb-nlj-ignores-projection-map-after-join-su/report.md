# INTERNAL Error (DataChunk::Slice assertion) in nested loop join above an outer join with scalar subqueries in its ON clause

> Note for the owner. DuckDB's `AI_POLICY.md` says "Autonomous agents are not allowed to be used for
> contributing to this project" and AI should not write the communication. Use this packet as working
> notes and write the issue yourself. The issue template (`.github/ISSUE_TEMPLATE/bug_report.yml`) also
> requires **Full Name** and **Affiliation**, which only you can fill in. `fix.diff` is for your reference
> only and should not be submitted as is.

**Trunk status:** still reproduces on current main `1b4c3e54ba` (2026-10-16, `v2.1.0-dev89158`). Not fixed upstream.

## What happens

The query below combines an outer join whose `ON` clause contains uncorrelated scalar subqueries with an
inner join on an inequality (`<>`) condition. Planning it as a nested loop join fails with an
assertion-backed `INTERNAL Error`, after which the database instance is invalidated. The query is valid
and should return 3 rows.

## To Reproduce

```sql
CREATE TABLE t0(c0 INTEGER);
INSERT INTO t0 VALUES (1);
SELECT 1 FROM t0 AS x1
  FULL JOIN t0 AS x2 ON ((SELECT 0) <> (SELECT count(*) FROM t0))
  JOIN (VALUES (7),(8),(9)) AS x3(c0) ON (x2.c0 <> x3.c0);
```

Expected: 3 rows of `1`. The FULL JOIN condition is `0 <> 1`, which is true, so the join yields the single
pair (1, 1). `1 <> x3.c0` then holds for all three rows of `x3`.

Actual (relassert build, i.e. `-DFORCE_ASSERT=1`):

```
INTERNAL Error: Assertion triggered in file ".../src/common/types/data_chunk.cpp" on line 386: other.ColumnCount() <= col_offset + ColumnCount()
...
This error signals an assertion failure within DuckDB. This usually occurs due to unexpected conditions or errors in the program's logic.
For more information, see https://duckdb.org/docs/current/dev/internal_errors
```

The same as a sqllogictest (`repro/test_subquery_outer_join_condition_nlj.test`):

```
statement ok
CREATE TABLE t0(c0 INTEGER);

statement ok
INSERT INTO t0 VALUES (1);

query I
SELECT 1 FROM t0 AS x1 FULL JOIN t0 AS x2 ON ((SELECT 0) <> (SELECT count(*) FROM t0)) JOIN (VALUES (7),(8),(9)) AS x3(c0) ON (x2.c0 <> x3.c0);
----
1
1
1
```

`repro/repro.sh <path-to-duckdb-cli>` runs `repro/repro.sql`. It exits 1 if the INTERNAL error reproduces
and 0 if the 3 rows come back.

We only ran assertion-enabled builds. In a release build without `FORCE_ASSERT`, the `D_ASSERT` is compiled
out and the mismatched chunk layout goes through unchecked. We did not run a release build, so we can't say
whether that gives a crash, a wrong result or silent success.

## Expected behaviour / contract

This is the universal "no internal error" rule. DuckDB's own message for this class of error says:
"This error signals an assertion failure within DuckDB. This usually occurs due to unexpected conditions
or errors in the program's logic." (https://duckdb.org/docs/current/dev/internal_errors)

## Environment

- OS: Linux x86_64 (Ubuntu, kernel 6.8), gcc 13
- DuckDB version: `v2.1.0-dev89154 (Development Version) ba16e6f761` (main, 2026-10-16), built
  RelWithDebInfo with `-DFORCE_ASSERT=1` and extensions core_functions, json, icu
- Also checked on main `1b4c3e54ba` (`v2.1.0-dev89158`, 2026-10-16), same build flags: same assertion
- Client: CLI
- Not tested: release v1.5.6. We could not install it here. By `git tag --contains`, the suspected
  introducing commit (below) is in no release tag, so this is probably a regression on the 2.x dev line.
  It needs a run against 1.5.6 before anyone calls it a "regression since v1.5.6".

## Root cause (analysis, from reading the code with the fix below)

1. `RecursiveDependentJoinPlanner::PlanAnyJoinCondition` (`src/planner/binder/query_node/plan_subquery.cpp`)
   plans the uncorrelated subqueries of the outer join's `ON` clause onto the left child of that join.
   This widens the join's output with the subquery columns.
2. When the parent join's child is rewritten, `ColumnBindingRewrite::RewriteChild`
   (`src/optimizer/column_binding_replacer.cpp`, ~line 318) freezes a projection map on the parent join to
   hide the extra columns. This behaviour came in with 35fca1ca1d "Preserve projection selections when
   rewriting logical children" (PR #26304, 2026-09-30). We think that commit introduced the bug, but we did
   not bisect it by building.
3. The parent join has an inequality condition, so `PlanComparisonJoin`
   (`src/execution/physical_plan/plan_comparison_join.cpp`) plans it as a `PhysicalNestedLoopJoin`, or as a
   `PhysicalBlockwiseNLJoin` for arbitrary conditions. Those operators do not apply projection maps (the
   NLJ applies only the left map of a MARK join, as the comment added in PR #27081 says). The output chunk
   types are taken from the logical op (projected layout), but the operator writes left+right columns in
   full, so `DataChunk::Slice` sees more source columns than the target chunk has.

PR #27081 (merged 2026-10-13) narrowed the `left_projection_map.empty()` assertion on these paths, but
nothing stops a non-empty map from reaching them. Our repro fails on trunk after that PR.

## Suggested fix

When planning a join as an NLJ (non-MARK) or BNLJ (comparison and any-join), plan the operator on the
full child layouts. Clear the projection maps, set the op's types to the unprojected left+right layout,
and put a `PhysicalProjection` on top that selects the columns the projection maps asked for. Also, have
`ColumnBindingResolver` resolve non-MARK `LogicalAnyJoin` conditions against the full child layouts, since
that is what the BNLJ evaluates. `fix.diff` (written against ba16e6f761; `git apply --check` is clean on 1b4c3e54ba) does this through a new helper
`PhysicalPlanGenerator::PlanJoinWithoutProjectionMaps`, and adds the test above plus two more queries.

Checked against that patch, on relassert builds:

- the repro returns 3 rows on the patched build and fails on the unpatched one (rerun for this packet)
- per triage notes, not rerun for this packet: the new test fails on base and passes with the fix, and
  `test/sql/join/*` (181 files) and `test/sql/subquery/*` (106 files) pass on the patched build

Making the NLJ/BNLJ operators honour projection maps natively would be another approach (as the hash
join and range joins already do).
