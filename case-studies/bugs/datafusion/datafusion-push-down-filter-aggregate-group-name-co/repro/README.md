# Repro: push_down_filter rewrites a HAVING predicate to the wrong group key

    ./repro.sh /path/to/datafusion-cli

Exit codes: 1 = bug ("Invalid comparison operation: Utf8View > Int32"), 0 = correct (one row, a = 2),
2 = cannot decide.

`repro.sql` sets `datafusion.optimizer.skip_failed_rules = true`. Without it, current `main` fails earlier in
`eliminate_group_by_constant` ("Projections require unique expression names"), a separate bug fixed by open PR #26356.
With #26356 applied, the bug shows on default settings (drop the SET line).

Results (2026-10-21, debug builds of datafusion-cli):
- main 3bc3d0bb822f: exit 1
- 55.2.0: exit 1
- main + PR #26356 (commits f7b82a342, 2a7c2fae3, 99902be5a): exit 1, and the query without the SET fails the same way
- main + PR #26356 + ../fix.diff: exit 0, and the query without the SET returns 2
- sqlite 3.45.1 (`CAST(a AS VARCHAR)`): 2

`../fix.diff` applies to plain main 3bc3d0bb. On main + PR #26356 + fix.diff, `cargo test -p datafusion-optimizer --lib`:
945 passed, 0 failed (includes the new test `filter_not_pushed_through_ambiguous_agg_group_name`).
