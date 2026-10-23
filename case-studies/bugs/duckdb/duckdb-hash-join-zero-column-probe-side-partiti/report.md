<!-- Owner note: DuckDB's AI_POLICY.md says "AI should not be used to generate comments when communicating with
maintainers" and "Autonomous agents are not allowed to be used for contributing to this project". Use this packet
as notes and write the issue yourself. The bug_report.yml template also requires Full Name and Affiliation. -->

# INTERNAL Error "GetPartitioningSpaceRequirement: tuple width should not be 0" for a hash join whose probe side has no columns

Not fixed upstream: reproduces on `main` at 1b4c3e54ba (the newest commit on main, fetched 2026-10-16) and on release v1.5.6.

## What happens

With `disabled_optimizers='unused_columns'`, the query below plans a SEMI hash join whose probe side is an
`UNGROUPED_AGGREGATE` with no aggregates, so the probe side has zero columns. During finalize of the hash join
build, `PhysicalHashJoin::PrepareFinalize` sizes the probe-side spill. It computes a row width of 0 for the probe
types and throws an InternalException. The database instance is then invalidated, and every later statement
fails with FATAL until restart.

Plan (`EXPLAIN`, trunk 1b4c3e54ba):

```
PROJECTION  Projections: 0
  HASH_JOIN  Join Type: SEMI; Conditions: 0 = #0
    UNGROUPED_AGGREGATE
      SEQ_SCAN t1
    PROJECTION  Projections: 0
      SEQ_SCAN t1
```

## To Reproduce

```sql
CREATE TABLE t1(c0 INTEGER);
SET disabled_optimizers='unused_columns';
SELECT 0 FROM t1 HAVING 0 IN (SELECT 0 FROM t1);
SELECT 42 AS still_alive;
```

Expected: the third statement returns **0 rows**. The ungrouped aggregate over the empty `t1` yields one row,
`0 IN (SELECT 0 FROM t1)` is false over an empty subquery, so `HAVING` removes that row. Trunk returns the same
0 rows when the `SET` line is left out, and the patched build returns 0 rows with it. The fourth statement
should return `42`.

Actual (trunk 1b4c3e54ba, v1.5.6 behaves the same):

```
INTERNAL Error: GetPartitioningSpaceRequirement: tuple width should not be 0
This error signals an assertion failure within DuckDB. This usually occurs due to unexpected conditions or errors in the program's logic.
For more information, see https://duckdb.org/docs/current/dev/internal_errors
FATAL Error: Failed: database has been invalidated because of a previous fatal error. The database must be restarted prior to being used again.
Original error: "GetPartitioningSpaceRequirement: tuple width should not be 0"
```

As a sqllogictest (this is the test added with the suggested fix):

```
statement ok
CREATE TABLE t1(c0 INTEGER);

statement ok
SET disabled_optimizers='unused_columns';

query I
SELECT 0 FROM t1 HAVING 0 IN (SELECT 0 FROM t1);
----
```

`repro/repro.sh /path/to/duckdb` runs `repro/repro.sql` and exits 1 when the error appears and 0 when `42` is printed.

## Which rule this breaks

DuckDB's own text for this error class: "This error signals an assertion failure within DuckDB. This usually occurs
due to unexpected conditions or errors in the program's logic."
(https://duckdb.org/docs/current/dev/internal_errors). `disabled_optimizers` is a documented user setting:
"Disable a specific, comma-separated set of optimizers." (`src/common/settings.json`). Turning off an optimizer
should not make a valid query hit an internal error and invalidate the database.

## Versions tested

| build | result |
|---|---|
| main 1b4c3e54ba3625bac56ad184f0ce642a8d2bfbd5 (`v2.1.0-dev89158`), RelWithDebInfo + FORCE_ASSERT, Linux x86_64, CLI | reproduces |
| v1.5.6 (069cc9f9b5) official Linux CLI binary | reproduces |
| main ba16e6f761 + this fix (plus unrelated local patches) | does not reproduce; `still_alive` = 42 |

OS: Linux x86_64 (Ubuntu, kernel 6.8). Client: CLI.

## Root cause

`src/execution/operator/join/physical_hash_join.cpp`:

- `PhysicalHashJoin::PrepareFinalize` (around line 1057 on trunk) calls
  `GetPartitioningSpaceRequirement(context, children[0].get().GetTypes(), ...)`. The external-join
  `FinishEvent` path (around line 1453) does the same with `op.types`.
- `GetTupleWidth` builds a `TupleDataLayout` from those types and returns `GetRowWidth()`. With zero probe
  columns the width is 0, and `GetPartitioningSpaceRequirement` throws `"tuple width should not be 0"`.
- The probe side is not spilled with just the probe columns. `HashJoinGlobalSinkState::probe_types`
  (around lines 460-461) is the probe columns plus a `HASH` column, and `ProbeSpill` is created with
  `probe_types`. The sizing therefore uses the wrong type list. With the real spill layout the width is never 0.

According to `git log`, the throw arrived in 70cad16983 ("counting fix", 2026-03-31). That commit replaced a
hand-summed width, which always added `sizeof(hash_t)`, with `TupleDataLayout::GetRowWidth()`. The commit is
in v1.4.5 and v1.5.2 through v1.5.6. Only v1.5.6 and trunk were run.

## Suggested fix

Size the probe-side requirement with `probe_types` (probe columns plus HASH), the layout that is actually
spilled, at both call sites. For ordinary joins this raises the estimate by 8 bytes per tuple. The patch is in
`fix.diff` and includes the sqllogictest
`test/sql/join/hash_join/test_hash_join_zero_probe_columns.test`, which also covers a non-empty probe side and
`debug_force_external=true`. It applies cleanly to trunk 1b4c3e54ba (`git apply --check`). The triage run reported
that `test/sql/join/hash_join/*`, `test/sql/join/external/*`, the semi/anti and mark join tests, and the new test pass. I did not re-run
those tests for this write-up.

## Side observations (not covered by this fix; re-run on trunk 1b4c3e54ba)

1. With a row in `t1` (`INSERT INTO t1 VALUES (1)`) the same query fails differently, on the assertion build:
   `INTERNAL Error: Assertion triggered in file ".../expression_executor.cpp" on line 32: exprs.size() > 0`
   (through `PhysicalExpressionScan`, a `LogicalExpressionGet` with zero expressions). That build has
   `FORCE_ASSERT`. I did not check a release build.
2. Wrong result: `CREATE TABLE t2 AS SELECT 0 c0 FROM range(100000); SET disabled_optimizers='unused_columns';
   SELECT 0 FROM range(3) HAVING 0 IN (SELECT c0 FROM t2);` returns 0 rows on trunk. Without the `SET` it
   returns 1 row (`0`), which is correct. Per triage, a RIGHT_SEMI hash join with a zero-column build side
   emits nothing. This probably deserves its own issue.
