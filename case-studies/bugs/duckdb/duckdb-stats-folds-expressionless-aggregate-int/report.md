# INTERNAL Error `exprs.size() > 0` (expression_executor.cpp) when statistics propagation folds an aggregate with no aggregate expressions (`HAVING` without aggregates)

> Note for the owner: duckdb's `AI_POLICY.md` says "AI should not be used to generate comments when communicating
> with maintainers" and "Autonomous agents are not allowed to be used for contributing to this project". Use this
> packet as notes and write the issue yourself. The bug template (`.github/ISSUE_TEMPLATE/bug_report.yml`) also asks
> for your Full Name and Affiliation. `fix.diff` is here for reference only; the project does not accept
> agent-made patches.

Not fixed upstream: it still reproduces on `main` at 1b4c3e54ba3625bac56ad184f0ce642a8d2bfbd5, which was the head of
`main` on 2026-10-16.

## What happens

With an assertion-enabled build, a valid query fails during physical planning:

```
INTERNAL Error: Assertion triggered in file ".../src/execution/expression_executor.cpp" on line 32: exprs.size() > 0
This error signals an assertion failure within DuckDB. This usually occurs due to unexpected conditions or errors in the program's logic.
```

The same query returns `1` with `SET disabled_optimizers='statistics_propagation'`. The v1.5.6 release CLI (assertions
compiled out) also returns `1`.

## To Reproduce

```sql
CREATE TABLE t0(c0 INTEGER);
INSERT INTO t0 VALUES (1);
SELECT (SELECT 1 FROM (SELECT 1 AS a FROM t0 HAVING true) WHERE a = (SELECT x.a1)) FROM (SELECT 1 AS a1) x;
```

- Expected: one row with the value `1`.
- Actual (trunk, `RelWithDebInfo` + `FORCE_ASSERT=1`): the INTERNAL Error above.

A shorter trigger, though it needs a non-default setting:

```sql
CREATE TABLE t0(c0 INTEGER);
INSERT INTO t0 VALUES (1);
SET disabled_optimizers='unused_columns';
SELECT 1 FROM t0 HAVING true;   -- expected 1; actual: the same INTERNAL Error
```

An empty `t0` does not trigger it, because then there are no partition stats to fold.

As a sqllogictest (`repro/statistics_aggregate_without_expressions.test`):

```
statement ok
CREATE TABLE t0(c0 INTEGER);

statement ok
INSERT INTO t0 VALUES (1);

query I
SELECT (SELECT 1 FROM (SELECT 1 AS a FROM t0 HAVING true) WHERE a = (SELECT x.a1)) FROM (SELECT 1 AS a1) x;
----
1

statement ok
SET disabled_optimizers='unused_columns';

query I
SELECT 1 FROM t0 HAVING true;
----
1
```

`repro/repro.sh <duckdb-cli>` runs the first query. It exits 1 if the INTERNAL error appears and 0 if the query
returns `1`.

## Expected behaviour (contract)

- `AGENTS.md`, Error Handling: "Use `D_ASSERT` for programmer errors (never triggered by user input)"
  (https://github.com/duckdb/duckdb/blob/main/AGENTS.md). Here a plain SQL query triggers one.
- `AGENTS.md`, Optimizer: optimizers "Transforms logical plans without changing semantics". Here the query works
  when `statistics_propagation` is disabled.
- The error text itself says it "signals an assertion failure within DuckDB"
  (https://duckdb.org/docs/current/dev/internal_errors).

## Environment

- OS: Linux x86_64 (Ubuntu, kernel 6.8)
- DuckDB version: v2.1.0-dev89158, commit 1b4c3e54ba (`main`), built with
  `-DCMAKE_BUILD_TYPE=RelWithDebInfo -DFORCE_ASSERT=1` (gcc)
- Client: CLI
- v1.5.6 release CLI: returns `1`. A release build has no asserts, so this does not show whether the code path runs there.

## Root cause

`StatisticsPropagator::TryExecuteAggregates` (`src/optimizer/statistics/operator/propagate_aggregate.cpp`) replaces
an ungrouped aggregate whose aggregates are all `COUNT(*)`/`MIN`/`MAX` with a `LogicalExpressionGet` computed from
partition stats. It checks `aggr.groups.empty()` and then loops over `aggr.expressions` to check that every aggregate
qualifies. When the aggregate has **no** expressions, which is what `SELECT 1 AS a FROM t0 HAVING true` produces, the
loop passes trivially. The function then builds a `LogicalExpressionGet` with zero types and one empty row.
`PhysicalPlanGenerator::CreatePlan(LogicalExpressionGet&)` constant-folds that row through
`PhysicalExpressionScan::EvaluateExpression`, which builds an `ExpressionExecutor` over an empty expression list and
hits `D_ASSERT(exprs.size() > 0)`.

Normally `RemoveUnusedColumns` gives such an aggregate a `COUNT(*)`, so it no longer has zero expressions. That does
not happen in the decorrelated subquery plan above, or whenever `unused_columns` is disabled. The folding came in with
f8e978251e ("... use that to compute COUNT(*) from stats", 2024-12). We did not bisect this; the commit is inferred
from the code.

## Suggested fix

In `TryExecuteAggregates`, return early when `aggr.expressions` is empty. The aggregate then stays as it is and still
produces the single row an ungrouped aggregate must return. `fix.diff` makes that four-line change and adds the
sqllogictest above as `test/optimizer/statistics/statistics_aggregate_without_expressions.test`.

What was verified:
- The controller checked that the repro reproduces on the loop's base build (ba16e6f761 plus earlier local patches)
  and does not reproduce with the fix.
- Triage ran test/optimizer, test/optimizer/statistics, test/sql/aggregate/aggregates and test/sql/subquery on base
  and fixed builds with a subset sqllogictest runner, and per-file results matched.
- On trunk 1b4c3e54ba: TRUNK_FIX_RESULT
