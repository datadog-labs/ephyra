# Segfault in `dense_rank() OVER ()` over a RIGHT_ANTI hash join that returns its unmatched rows

> Note for the owner: duckdb's `AI_POLICY.md` says "AI should not be used to generate comments when communicating
> with maintainers" and "Autonomous agents are not allowed to be used for contributing to this project". Use this
> packet as notes and write the issue yourself. The bug template (`.github/ISSUE_TEMPLATE/bug_report.yml`) also asks
> for your Full Name and Affiliation. `fix.diff` is here for your reference only; the project does not accept
> agent-made patches.

## What happens

The query below kills the process with SIGSEGV. It should return one row (`NULL, 1`). With `SET threads = 1` it
crashed 5 out of 5 times on current `main`. With other thread counts it crashes in about half the runs.

## To Reproduce

```sql
SET threads = 1;
SELECT x, dense_rank() OVER ()
FROM (VALUES (7), (NULL)) t(x)
ANTI JOIN (VALUES (7), (2), (1)) s(y) ON x = y AND NULL::BOOLEAN
WHERE x IS NULL;
```

Expected (and what v1.5.6 returns):

```
┌───────┬──────────────────────┐
│   x   │ dense_rank() OVER () │
├───────┼──────────────────────┤
│  NULL │                    1 │
└───────┴──────────────────────┘
```

Actual on `main` @ 1b4c3e54ba: `Segmentation fault` (exit status 139), with no output.

`repro/repro.sh <duckdb-cli>` runs `repro/repro.sql` 5 times. It exits 1 if the process dies from a signal, 0 if
every run gives the correct row, and 2 if it cannot tell.

Backtrace (RelWithDebInfo build with `FORCE_ASSERT`, trunk 1b4c3e54ba, threads=1):

```
#0 TemplatedValidityMask<unsigned long>::GetValidityEntry (entry_idx=2199020978026, ...)
#1 WindowDenseRankExecutor::GetData (..., row_idx=0, ...)          src/function/window/window_rank_function.cpp
#2 WindowExecutor::Evaluate (..., row_idx=0, ..., count=0)
#3 WindowLocalSourceState::GetData                                 src/execution/operator/aggregate/physical_window.cpp
#4 WindowLocalSourceState::ExecuteTask
#5 PhysicalWindow::GetDataInternal
#6 PipelineExecutor::FetchFromSource
```

The plan has a `HASH_JOIN` with `Join Type: RIGHT_ANTI` feeding `WINDOW` (`dense_rank() OVER ()`).

## Expected behaviour / contract

This is the general rule that a valid query must not crash the process. The query uses only in-memory `VALUES`, no
extensions and no external files. v1.5.6 returns the expected row.

## Versions tested

| build | result |
|---|---|
| `main` @ `1b4c3e54ba3625bac56ad184f0ce642a8d2bfbd5` (v2.1.0-dev89158), Linux x86_64, CLI, RelWithDebInfo + FORCE_ASSERT, source build | SIGSEGV: threads=1 5/5; threads=2/4/16 crash in about half of 5 runs each |
| `main` @ `ba16e6f761` + three unrelated join patches (the loop's base build) | SIGSEGV (controller rerun) |
| v1.5.6 official `duckdb_cli-linux-amd64` release binary | no crash in 20 runs (threads=1 and 4), correct result |

When this was tested (2026-10-16), the latest `main` was 1b4c3e54ba. The suspect files have not changed since
ba16e6f761. I did not establish whether 1.5.x is truly unaffected. Its `NaturalSort::Combine` is reportedly the
same code, and the bad read depends on memory contents, so the release binary may simply not have hit it. I only
observed v1.5.6 not crashing. I did not build v1.5.6 from source or run it with a sanitizer.

## Root cause (analysis)

`NaturalSort` (`src/common/sort/natural_sort.cpp`) handles windows that have no PARTITION BY or ORDER BY. In
`NaturalSort::Combine` (line ~100), the first thread-local state to combine is adopted as the hash group's column
collection:

```cpp
} else {
    hash_group = make_uniq<NaturalSortGroup>(context.client);
    hash_group->columns = std::move(lstate.unsorted);
    hash_group->count += hash_group->columns->Count();
}
```

This happens even when that local collection is empty. `ColumnDataCollection::InitializeAppend` always allocates one
chunk, so an empty local collection still holds one zero-row chunk. Here the RIGHT_ANTI hash join's probe pipeline
sinks 0 rows into the window from one local state, and the outer-scan pipeline later sinks the one unmatched row
from another local state. The empty one combines first. The hash group then has 2 chunks for 1 row, and one of them
is a zero-row block. (`ColumnDataCollection::Combine` already skips an empty *other* collection, so only this
adoption path has the problem.)

The window source schedules a task for the zero-row block, so `WindowExecutor::Evaluate` is called with `count = 0`.
`WindowDenseRankExecutor::GetData` (`src/function/window/window_rank_function.cpp:293`) reads `peer_begin[0]` and
`partition_begin[0]` without checking the count whenever `lpeer.row_idx != row_idx`. With an empty bounds chunk those
values are uninitialised. They are used to index `order_mask`, which gives an out-of-bounds `GetValidityEntry`. According to triage,
other window functions in this shape (rank, row_number, aggregates) tolerate the empty block. I did not rerun that.

The `NaturalSort` code appears to have come in with the SortStrategy refactoring (PR #19814, merged 2025-11-17).

## Suggested fix

In `NaturalSort::Combine`, skip local states whose collection is missing or empty. Then an empty collection never
becomes the base of the hash group. If `gstate.count > 0`, some non-empty local state will still create the group.
`fix.diff` does this and adds `test/sql/window/test_natural_sort_empty_combine.test`, which runs the query above
with threads 1, 2 and 4.

Another option, which could be added as well: make `WindowDenseRankExecutor::GetData` (or `WindowExecutor::Evaluate`)
return early when `count == 0`, or stop the window source from scheduling zero-row blocks.

Verification of `fix.diff` (by the loop's controller and triage, not rerun in this write-up): it applies to `main`
@ 1b4c3e54ba (checked with `git apply --check`). On ba16e6f761 + earlier patches, the repro no longer crashes and the
query returns `NULL, 1` with threads 1/2/4. The unittest groups `test/sql/window/*` (101 cases),
`test/sql/join/asof/*` and `test/sql/copy/partitioned/*` pass. I did not rebuild 1b4c3e54ba with the fix.

## Environment

- OS: Linux 6.8 x86_64
- DuckDB: v2.1.0-dev89158 (1b4c3e54ba), built from source
- Client: CLI
