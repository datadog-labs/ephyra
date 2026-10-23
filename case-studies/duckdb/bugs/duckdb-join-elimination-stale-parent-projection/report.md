# INTERNAL Error "Projection map references a missing child column": join elimination leaves the parent filter's projection map stale

> Note for the owner: DuckDB's `AI_POLICY.md` says "AI should not be used to generate comments when communicating with
> maintainers" and "Autonomous agents are not allowed to be used for contributing to this project". Treat this as
> working notes. If you file it, write the issue yourself, and do not submit `fix.diff` as an agent-made PR. The
> sections below follow `.github/ISSUE_TEMPLATE/bug_report.yml`. Full Name and Affiliation are yours to fill in.

**Status:** reproduces on current `main` at `1b4c3e54ba` (2026-10-16 16:16 UTC; this was upstream HEAD when re-checked
at 21:48 UTC the same day). Not fixed upstream. Does not affect v1.5.6, which rejects the query with
`Not implemented Error: Cannot perform non-inner join on subquery!`.

## What happens?

A valid query with SELECT DISTINCT over a LEFT JOIN that join elimination can remove, plus a WHERE filter above a
second LEFT JOIN whose ON clause contains a scalar subquery, fails at planning time:

```
INTERNAL Error: Projection map references a missing child column
This error signals an assertion failure within DuckDB. ...
```

If `join_elimination` is disabled, the same query returns the correct result (`1`, `2`).

## To Reproduce

```sql
CREATE TABLE t0(c0 INTEGER);
INSERT INTO t0 VALUES (1), (2);
SELECT DISTINCT x1.c0
FROM t0 AS x1
LEFT JOIN t0 AS x2 USING (c0)
LEFT JOIN t0 AS x3 ON ((SELECT true) AND false)
WHERE true;
```

Expected (and what you get after `SET disabled_optimizers='join_elimination';`):

```
1
2
```

Actual (default settings):

```
INTERNAL Error: Projection map references a missing child column
```

The query passes if any one of `join_elimination`, `filter_pushdown`, `join_order` or `unused_columns` is disabled
(triage finding). It needs at least 2 rows in `t0`. sqllogictest version: `repro/join_elimination_projection_map.test`.

## Contract violated

DuckDB's internal-error message says the error "signals an assertion failure within DuckDB. This usually occurs due to
unexpected conditions or errors in the program's logic"
(`src/common/error_data.cpp`, linked from https://duckdb.org/docs/current/dev/internal_errors). The query is valid and
gives a correct answer once one optimizer is turned off. An optimizer pass must not change whether a query succeeds.

## Root cause

1. When subqueries are planned, `ColumnBindingRewrite::RewriteChild` (`src/optimizer/column_binding_replacer.cpp:327`,
   "Freeze the implicit selection before the callback can change the child's output layout") gives the WHERE
   `LogicalFilter` an explicit positional `projection_map` over all columns of its child, the LEFT JOIN with `x2`.
   This behaviour arrived with 35fca1ca1d / PR #26304 "Preserve projection selections when rewriting logical
   children" (2026-09-30). No release tag contains that commit.
2. `JoinElimination::TryEliminateJoin` (`src/optimizer/join_elimination.cpp:307-309`) sees that the DISTINCT makes the
   LEFT JOIN with `x2` redundant. It replaces the join with its outer child:
   `join_parent->children[pipe_info.join_index] = std::move(join_op->children[outer_idx]);`. The new child has fewer
   columns, but the parent's projection map is never updated, so it now points past the end of the child.
3. The next pass that reads the map, `RemoveUnusedColumns` (`src/optimizer/remove_unused_columns.cpp:332-333`),
   throws `InternalException("Projection map references a missing child column")`.

(Triage confirmed the order by checking every projection map after each optimizer pass. The first stale map appears
right after JOIN_ELIMINATION.)

## Suggested fix

In `TryEliminateJoin`, take the join's column bindings before the swap. After the swap, remap the parent's projection
map for that child from the old bindings to the outer child's bindings with the existing helper
`LogicalOperatorVisitor::RemapProjectionMap`, which clears the map if a selected inner column no longer exists. This
is an 8-line change in `fix.diff`, which also adds `test/optimizer/join_elimination_projection_map.test`.

Fix verification: FIX_VERIFICATION_PLACEHOLDER

## Environment

- OS: Linux x86_64 (Ubuntu, kernel 6.8)
- DuckDB version: v2.1.0-dev89158, source_id `1b4c3e54ba` (built from `main`, RelWithDebInfo + FORCE_ASSERT). The
  error is an unconditional `throw` and not a `D_ASSERT`, so release builds should hit it too (not tested on a
  nightly binary).
- Client: CLI
