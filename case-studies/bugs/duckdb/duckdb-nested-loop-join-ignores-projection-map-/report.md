# INTERNAL Error (assertion in plan_comparison_join.cpp) for an inequality join on top of a LEFT JOIN whose ON clause contains a subquery

> **Note for the owner:** this has the **same root cause** as the loop's packet
> `work/bugs/nlj-ignores-projection-map-after-join-su` (FULL JOIN with scalar subqueries in ON). Accepted
> patch `patches/001-…` fixes both. File **one** upstream issue and add the query below as a second
> reproduction; don't file a separate one. DuckDB's `AI_POLICY.md` says AI should not be used to write
> comments to maintainers, and autonomous agents may not contribute. So write the issue text yourself,
> and treat the patch as a pointer, not as a PR to submit.
>
> Not fixed upstream: it still reproduces on `main` @ `1b4c3e54ba` (2026-10-16, the latest commit when checked).

Template: `.github/ISSUE_TEMPLATE/bug_report.yml` (What happens / To Reproduce / OS / DuckDB Version /
Client / Hardware / Full Name / Affiliation / checkboxes). Fields below follow it. Name and affiliation
are left for the owner.

## What happens?

An inner inequality join (`<>`) above a LEFT JOIN whose ON condition contains an uncorrelated
subquery (`1 IN (SELECT 1)`) fails at physical planning:

```
INTERNAL Error: Assertion triggered in file ".../src/execution/physical_plan/plan_comparison_join.cpp" on line 276: op.left_projection_map.empty() || op.join_type == JoinType::MARK
```

This happens with default settings, and also when the table is empty. The same query with `ON (true)`
in place of `ON (1 IN (SELECT 1))` returns the correct 4 rows.

## To Reproduce

```sql
CREATE TABLE t0(c0 INTEGER);
INSERT INTO t0 VALUES (1), (2);
SELECT t0.c0, t2.c0 FROM t0 LEFT JOIN t0 t1 ON (1 IN (SELECT 1)) JOIN t0 t2 ON (t0.c0 <> t2.c0) ORDER BY ALL;
```

Expected (the LEFT JOIN condition is always true, so this equals the `ON (true)` form, which DuckDB answers
correctly on the same build):

```
1,2
1,2
2,1
2,1
```

Actual (relassert build, `main` @ 1b4c3e54ba):

```
INTERNAL Error: Assertion triggered in file ".../src/execution/physical_plan/plan_comparison_join.cpp" on line 276: op.left_projection_map.empty() || op.join_type == JoinType::MARK
This error signals an assertion failure within DuckDB. This usually occurs due to unexpected conditions or errors in the program's logic.
```

`PRAGMA disable_optimizer;` gives the same error, so the problem is in the planner, not an optimizer.
`repro/repro.sh <duckdb-cli>` runs this and exits 1 when it reproduces. A sqllogictest version is in
`repro/test_nlj_projection_map_left_join_in_subquery.test`.

Only checked on assertion-enabled builds (`FORCE_ASSERT`, i.e. `make relassert`). Not run on a release
build or on v1.5.6. Without assertions, triage saw that the operator's declared types no longer match its
output, which gives `DataChunk`/`Vector::Reference` internal errors and possibly wrong columns. That
observation came from triage on the loop's base commit ba16e6f761; this writeup did not re-run it.

## Contract

No documented behaviour allows an internal error for valid SQL. DuckDB's own text, printed with the error
(https://duckdb.org/docs/current/dev/internal_errors): "This error signals an assertion failure within
DuckDB. This usually occurs due to unexpected conditions or errors in the program's logic."

## Environment

- OS: Linux x86_64 (Ubuntu, kernel 6.8)
- DuckDB Version: v2.1.0-dev89158, source_id `1b4c3e54ba` (main, 2026-10-16). Also on `ba16e6f761`.
- Client: CLI
- Build: `RelWithDebInfo -DFORCE_ASSERT=1`, gcc 13, extensions core_functions;parquet;json;icu;tpch

## Root cause (from triage, read in the source; not bisected)

- `Binder::CreatePlan(BoundJoinRef)` builds a `LogicalAnyJoin` for the non-inner join with a subquery in its
  condition. The dependent-join planner (`PlanAnyJoinCondition` / `PlanJoinSubqueries`) plans the uncorrelated
  subquery into the join's left child, which adds a column to that child's output.
- When the parent's children are rewritten (`ColumnBindingRewrite::RewriteChild`,
  `src/optimizer/column_binding_replacer.cpp`, added in 35fca1ca1d "Preserve projection selections when
  rewriting logical children", 2026-09-30) and `RemapProjectionMapStrict` runs, the parent inequality join
  gets a non-empty `left_projection_map`.
- `PlanComparisonJoin` picks `PhysicalNestedLoopJoin` (or `PhysicalBlockwiseNLJoin`) for a `<>` condition.
  Neither operator applies projection maps. Until now only the equality-join path (hash join) ever received
  maps from column lifetime analysis. PR #27081 (d165437468) added the assertion, which states this
  assumption, and the planner breaks it here.
- The introducing commit was not bisected. Triage found no release tag containing 35fca1ca1d, so released
  versions are probably not affected (not tested).

## Suggested fix

In the physical planner, plan NLJ/BNLJ joins without their projection maps and apply the maps with a
`PhysicalProjection` on top. The MARK NLJ is the exception: it applies its left map itself, so that map
stays. Do the same in `plan_any_join.cpp`. There, `ColumnBindingResolver` must resolve the ANY-join
condition against the unprojected child layouts. `fix.diff` (the loop's accepted patch 001) does this and
adds `test/sql/subquery/scalar/test_subquery_outer_join_condition_nlj.test`.
`alt-fix-from-triage.diff` is triage's version of the same approach. It no longer applies on top of
patch 001.

Fix verification: see "Fix check" below.

## Fix check

(pending: trunk 1b4c3e54ba + fix.diff, CLI)
