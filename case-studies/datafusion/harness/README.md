# dfz — DataFusion harness

`build.sh <src> <build>` builds `crate/` (Rust, depends on `<src>/datafusion/core` by path and on ephyra)
with profile `fuzz` (release + debug assertions, no LTO) into `<build>/bin/dfz`.
`run.sh <build> <seed> <secs> <out>` runs `dfz search`: a supervisor that generates cases from the seed and
sends them (one JSON line each) to a long-lived `dfz worker` child. A worker that dies or gives no verdict
within 30 s is killed and restarted; that is reported as a crash/hang. Findings are reduced (other variants
dropped, then table rows dropped greedily while the signature still reproduces) and written as
`findings/<sig>/<n>/{finding.json,case.json,case.sql,repro.sh}`. `repro.sh <build>` runs
`dfz repro case.json <sig>` (5 attempts, because multi-partition execution is nondeterministically scheduled).

`build.sh <src> <build> coverage` builds the same binary with `-C instrument-coverage` (explicit `--target`
so build scripts and proc macros stay uninstrumented) and writes `<build>/coverage.json` with the binary and
the toolchain's `llvm-profdata`/`llvm-cov`. The worker inherits the environment (incl. `LLVM_PROFILE_FILE`);
at the end of a run the supervisor closes the worker's stdin so it exits normally and flushes its profile
(the search stops min(5 s, 5%) before the deadline to leave time for that). Workers killed after a hang or
crash lose their profile.

Other commands: `dfz gen <seed> <n>` prints generated queries; `dfz run case.json` judges one case in-process.

## Generator (`crate/src/casegen.rs`)

1–3 tables `t0..t2`, 2–4 columns (`c0` always an integer key; BIGINT/INT/VARCHAR/DOUBLE/BOOLEAN), 0–10 rows
from small shared value pools (ints -2..4, 7 short strings, a few floats incl. NaN, 15% NULL). Each table is a
`MemTable` with 1–3 partitions and 1/2/3/100-row batches. Queries are typed ASTs rendered to SQL: joins
(inner/left/right/full/cross/left semi/left anti; equi, multi-key, range-only and arbitrary ON), derived
tables, UNION/INTERSECT/EXCEPT [ALL], aggregates (count/sum/min/max/avg/bool_and/bool_or/string_agg,
DISTINCT, FILTER) with GROUP BY / ROLLUP / CUBE / GROUPING SETS / HAVING, peer-order-independent windows
(rank, dense_rank, aggregates with the default RANGE frame), scalar/EXISTS/IN/NOT IN/ANY/ALL subqueries
(correlated), CASE/COALESCE/NULLIF/BETWEEN/IN/LIKE/ILIKE/IS [NOT] DISTINCT FROM, string functions, casts.
ORDER BY at top level or in subqueries always covers every output column, so LIMIT/OFFSET results are
determined.

## Oracles (`crate/src/exec.rs`)

1. **toggle** (config differential). Each case runs under `base` (defaults, `target_partitions=4`) and 3
   random variants, each documented in `datafusion/common/src/config.rs` as a performance/plan-shape knob:
   target_partitions 1/3/16, batch_size 1–3, prefer_hash_join=false (SMJ), enable_piecewise_merge_join,
   all repartition_* off, join reordering off, dynamic filter pushdown off, partitioned hash join
   (single-partition thresholds 0), skip partial aggregation always, join integer prefilter on, top-k
   rules off, coalesce_batches off, max_passes=1, or `remove_optimizer_rule(<logical rule>)` (25% of variants).
   Rows are compared as sorted multisets (as sequences when the top-level ORDER BY covers all columns).
   - `toggle__rows_<variant>`: both succeed, rows differ.
   - `toggle__error_<variant>`: exactly one of base/variant errors.
2. **crash**: `crash__panic_<file>` (panic hook; includes debug_assert and panics in tokio tasks),
   `crash__internal_error__<area>` ("Internal error"; area = first plain words of the message, e.g.
   `physical_input_schema_should`; `repro` given the old coarse `crash__internal_error` matches any area), `crash__optimizer_rule_failed__<rule>` (rule = the failing optimizer rule's name; `repro` given the old coarse `crash__optimizer_rule_failed` matches any rule; with `skip_failed_rules=false`
   the docs say rule errors fail the query; the generator only builds valid queries), `crash__hang`,
   `crash__abort` / `crash__stack_overflow` (worker died).

## Known differences masked (each counted in stats.json)

- `skip_both_error` / `note_both_error`: base and variant both error (unsupported SQL shapes, ~6–13%).
- `note_rule_removed_error`, `note_rule_removed_internal`: a variant that removes an optimizer rule (or
  sets max_passes=1) errors while base succeeds, e.g. "Physical plan does not support logical expression
  Exists" or "Internal error: coalesce should have been simplified to case". Some rules are required for
  physical planning; this is not a correctness claim. Wrong *rows* under rule removal are still reported.
- `note_resource_error`: "Resources exhausted".
- Values avoid -0.0 (grouping representative of 0.0/-0.0 is unspecified; -0.0 computed by arithmetic is rendered as 0.0), float SUM (addition order), and
  integer overflow / division by zero (divisors are wrapped in NULLIF(x, 0)).
- Subqueries inside aggregate arguments and join ON clauses are generated only 5% of the time: they hit
  frequent optimizer-rule schema errors (real, but they would dominate the failure rate).

## Throughput

~60 cases/s for a single run.sh (each case = 4 query executions, 4 tokio worker threads); ~46 cases/s
each with 8 runs in parallel on 16 CPUs.

## Own gate run (2026-10-16, 4 x 240 s, seeds 1-4)

- base: ~11k cases per run, failure rate 0.30-0.44%, mostly `crash__optimizer_rule_failed` (subqueries in
  aggregate args / join ON: schema errors) and `crash__panic_topk_mod` (`assert!(k > 0)` with LIMIT 0 when
  eliminate_limit is removed); also SanityCheckPlan errors under tp1/batch_small, a nullability internal
  error, and a few wrong-row candidates (smj, skip_partial_agg, tp1, no_dynamic_filter).
- mutants: all 4 caught in every one of the 4 runs (findings that repro on the mutant and not on base).
  01: ~200 failures/run; 02: 44-57 `toggle__rows_smj`/run; 03: 1-5 `toggle__rows_batch_small`/run;
  04: 2-5 rows mismatches under no_rule_{push_down_filter,eliminate_outer_join,eliminate_filter}.

## Mutants (`mutants/*.diff`)

01 SortPreservingMerge fetch off-by-one (one extra row under LIMIT); 02 SMJ treats NULL join keys as equal;
03 OFFSET not carried across input batches; 04 push_down_filter treats RIGHT JOIN's left side as preserved.
