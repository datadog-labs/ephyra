# Join order optimizer drops a filter that references no column (`WHERE NULL`, `WHERE random() > 2`) when filter_pushdown is disabled

> Note for the owner: duckdb's `AI_POLICY.md` says "AI should not be used to generate comments when communicating with
> maintainers" and "Autonomous agents are not allowed to be used for contributing to this project". This packet is
> background material. If you file it, write the issue yourself. The bug template also asks for **Full Name** and
> **Affiliation**. `fix.diff` is here so you can see the root cause; it is not meant to be sent as a PR.

## What happens?

If a filter predicate references no column of any relation (a constant like `WHERE NULL`, or something volatile
like `WHERE random() > 2`) and ends up between reorderable joins, the join order optimizer removes it from the plan.
A simple way to get there: disable `filter_pushdown`. A query with two uncorrelated scalar subqueries in the SELECT
list then returns its rows unfiltered. `WHERE NULL` returns a row, and `WHERE random() > 2` returns every row.

With default settings the same query gives the correct result (0 rows). So the wrong plan only shows up with
`disabled_optimizers` set. `disabled_optimizers` is a regular (non-debug) setting:
"Disable a specific, comma-separated set of optimizers." Disabling an optimizer should not change query results.

## To Reproduce

```sql
CREATE TABLE t0(c0 INTEGER);
INSERT INTO t0 VALUES (1);
SET disabled_optimizers='filter_pushdown';
SELECT (SELECT 1) AS a0, (SELECT 2) AS a1 FROM t0 WHERE NULL;
```

Expected: 0 rows. `WHERE NULL` keeps no rows, and with default settings DuckDB returns 0 rows.

Actual (v2.1.0-dev89158, 1b4c3e54ba, and v1.5.6):

```
┌───────┬───────┐
│  a0   │  a1   │
├───────┼───────┤
│     1 │     2 │
└───────┴───────┘
```

Other forms I reran on the same build with `filter_pushdown` disabled:

| query | actual | expected (= default settings) |
|---|---|---|
| `SELECT count(*) FROM (SELECT (SELECT 1),(SELECT 2) FROM t0 WHERE random() > 2)` | 1 | 0 |
| `SELECT min((SELECT 0 FROM t0)), count((SELECT 0 FROM t0)) FROM t0 JOIN t0 AS x2 ON CAST(NULL AS BOOLEAN)` | 0, 1 | NULL, 0 |

If you also disable `filter_pullup` or `join_order` (`SET disabled_optimizers='filter_pushdown,filter_pullup'`),
you get the correct result (0 rows). `WHERE false` is not affected because it is folded earlier.

`EXPLAIN` (optimized) on the minimal query shows no filter anywhere: just Projection → Cross Product(Seq Scan t0,
Cross Product(Projection 1 / Dummy Scan, Projection 2 / Dummy Scan)).

## Contract violated

- Universal rule: a `WHERE` clause keeps only the rows where the condition is true. `WHERE NULL` keeps none.
- `AGENTS.md`, Optimizer section: the optimizer "Transforms logical plans without changing semantics"
  (https://github.com/duckdb/duckdb/blob/1b4c3e54ba3625bac56ad184f0ce642a8d2bfbd5/AGENTS.md#L161).
- Disabling an optimizer (`disabled_optimizers`) gives a different result from the default plan for the same query.

## Root cause

With `filter_pushdown` disabled, the `WHERE` filter stays in a `LogicalFilter`. `filter_pullup` then lifts it above
the cross products built for the uncorrelated scalar subqueries, which places it between reorderable joins. Then
`RelationManager::ExtractEdges` (`src/optimizer/join_order/relation_manager.cpp`, around line 1063) goes through
the expressions of every `LogicalFilter` in the join tree:

```cpp
ExtractBindings(*expression, bindings);
if (bindings.empty()) {
    // the filter is on a column that is not in our relational map. (example: limit_rownum)
    // in this case we do not create a FilterInfo for it. (duckdb-internal/#1493)
    continue;
}
```

An expression that references no relation gets no `FilterInfo`. Later, `QueryGraphManager::Reconstruct` rebuilds
the join tree from relations and `FilterInfo`s only. The original `LogicalFilter` operator is kept only if it sits
in the operator chain above the root join, or inside a relation. A relation-free filter in between is therefore lost.

## Suggested fix

In that `bindings.empty()` branch, keep the expression if its filter operator will be dropped during
reconstruction: the operator is not in the single-child chain above the root join, and it is not part of a
non-reorderable relation (that case covers the existing limit_rownum / #1493 handling). Then push the kept
expressions on top of the reconstructed join tree in `QueryGraphManager::Reconstruct`. The filters above the
root join have to be excluded, or they would be applied twice. A first version that did not exclude them failed
`test/sql/subquery/scalar/test_correlated_having.test` (the HAVING `nextval` case).

`fix.diff` implements this (+ `test/optimizer/joins/join_order_constant_filter.test`). It applies cleanly to
1b4c3e54ba. On a build of ba16e6f761 with this patch (plus unrelated earlier fixes), the minimal repro returns
0 rows. The triage notes say `test/optimizer/*`, `test/sql/subquery/*`, `test/sql/join/*`, `test/sql/filter/*` and
`test/sql/aggregate/*` pass on that build. I did not rerun the unit tests for this report.

## OS / Version / Client

- OS: Linux x86_64 (Ubuntu, kernel 6.8)
- DuckDB: main @ `1b4c3e54ba3625bac56ad184f0ce642a8d2bfbd5` (v2.1.0-dev89158), relassert build; also reproduced with
  the released v1.5.6 CLI (069cc9f9b5). So this is not a regression since 1.5.6.
- Client: CLI

## sqllogictest

`repro/join_order_constant_filter.test` (the same file is in `fix.diff`).
