# duckdb harness (ephyra)

One Rust binary, `duckharness`, built by `build.sh` next to a relassert DuckDB build
(`RelWithDebInfo -DFORCE_ASSERT=1`, extensions core_functions/json/icu (+parquet, always built)).

- `build.sh <src> <build>`: cmake/ninja into `<build>/cmake` (targets `src/libduckdb.so`, `duckdb`), then
  `cargo build` into `<build>/cargo`, binary copied to `<build>/duckharness`. ccache is shared across trees
  (`CCACHE_BASEDIR`, `CCACHE_NOHASHDIR`).
- `run.sh <build> <seed> <secs> <out>`: one search process. `stats.json` is rewritten every ~2 s.
- findings: `<out>/findings/<sig>/<n>/{finding.json, case.json, case.sql, repro.sh}`; `case.sql` is the
  same case as a plain SQL script for the duckdb CLI (`<build>/cmake/duckdb -no-agent < case.sql`).
  `repro.sh <build>` re-runs the case 3x on a fresh database: exit 1 = same signature, 0 = not, 2 = undecided.

## Coverage build

`build.sh <src> <build> coverage` builds the same configuration with clang 18
(`-fprofile-instr-generate -fcoverage-mapping`, profile runtime linked into `libduckdb.so`) and writes
`<build>/coverage.json` (object: `<build>/cmake/src/libduckdb.so`; tools: `/usr/lib/llvm-18/bin/llvm-{profdata,cov}`).
The harness binary itself is not instrumented. run.sh passes the environment through; the worker child
exits cleanly at the end of a run (stdin closed, up to 10 s grace) so it writes its profile to
`$LLVM_PROFILE_FILE`; workers killed after a timeout or crash write none. A build dir configured for the
other mode, or from another source tree, is reconfigured (cmake caches the source dir; reusing it once
made the coverage map point at the wrong tree). In coverage mode ccache path rewriting is off. ~19 cases/s on the coverage build; 40 s gave 22 % line coverage of libduckdb.so.

## Architecture

The search process generates a case (ephyra generator, custom shrinker), renders it to SQL and sends the
statements to a **worker child** (`duckharness worker`: the same binary, linked to `libduckdb.so` of the
build under test through the C API, one JSON request/response per line). A segfault/abort kills only the
worker (`crash__signal`); a request without answer in 30 s is `crash__hang` (the worker is killed and
restarted). The database is reused between cases (open costs ~28 ms, a statement ~1 ms): tables are
dropped, settings reset; it is reopened after any INTERNAL error (which invalidates the instance).

## Oracles

1. **optdiff** (optimizer toggle differential, ~50 % of cases). The same query on the same data runs twice:
   with default settings, and with a variant: `SET enable_optimizer=false` (20 %) or
   `SET disabled_optimizers='<1-4 random names>'` (80 %; names from `duckdb_optimizers()`).
   Contract: optimizers must not change results ("no wrong result"; `disabled_optimizers`: "Disable a
   specific, comma-separated set of optimizers", settings.json:719). Compared: column names and type ids
   (`optdiff__schema`), row multiset (`optdiff__rows`), or the row sequence when the top-level query has an
   ORDER BY (the generator always orders by every output column, so the order is total up to identical rows).
   Signature `optdiff__rows_<tag>` / `optdiff__schema_<tag>`, tag = the single disabled optimizer the
   shrinker localized (`NoOpt -> all optimizers disabled -> halves -> single names`), `multi` when it could
   not get below several, `noopt` when only `enable_optimizer=false` shows it. The tag says which pass,
   when toggled, changes the result; the bug may be in that pass or in a pass that depends on it.
2. **crash** (always on): any `INTERNAL Error` (D_ASSERT throws `InternalException` in relassert, so
   `crash__assert_<source file stem>` when the message has "Assertion triggered", else
   `crash__internal_<first words of the message up to the first value>`), FATAL errors
   (`crash__fatal`), worker death (`crash__signal`), default-settings query > 30 s (`crash__hang`), in setup
   statements, the default query and the variant query.
3. **explainsql** (EXPLAIN (SQL) round-trip, ~25 % of cases). The default run executes the query; the variant
   sets `debug_verify_statement='explain_sql'`, which replaces the statement by the SQL that `EXPLAIN (SQL)`
   exports from its optimized plan (unsupported shapes fall back to the original). Both runs share a base
   `disabled_optimizers` of 0-3 random passes (other plan shapes get exported). Contract:
   `src/planner/sql_export/README.md`: "The generated query preserves SQL semantics when rebound under the same
   catalog, settings, extensions, and external environment. It preserves output column names, column order,
   logical types, NULLs, duplicates". Signatures: `explainsql__rows_<area>` / `explainsql__schema_<area>`
   (area = rarest construct in the shrunk query: window, setop, quantified, exists, subquery, cte, semi_anti,
   outer_join, aggregate, distinct, limit, join, scan) and `explainsql__error_<kind>_<message words>` when the
   exported SQL fails with a bind/parse/catalog/... error while the original succeeds (runtime errors such as
   conversion/out-of-range are masked as `err_mismatch`, as for optdiff).
4. **debugdiff** (~20 % of cases): the variant is one debug setting that changes execution strategy or vector
   representation and must not change results (`gen::DEBUG_VARIANTS`: `debug_verify_vector` modes,
   `debug_force_external`, `debug_window_mode`, `debug_force_no_cross_product`, `debug_verification_projection`,
   scan strategies, `debug_force_fetch_row`, caching operators off, `debug_order_verification`, threads 1/8;
   each is a whole-suite config in `test/configs/`). Signatures `debugdiff__rows_<tag>` / `debugdiff__schema_<tag>`.
5. **self-checking settings** (20 % of cases, both runs): one of `debug_verify_stats`, `debug_verify_serializer`,
   `debug_verify_column_bindings`, `debug_verification_mode` verify_vectors/verify_functions,
   `debug_verify_aggregate_state_export`; violations surface through the crash oracle.

## Generator

Schema: 1-3 tables `t0..t2`, 1-4 columns of INTEGER/VARCHAR/BOOLEAN/DOUBLE, shared small value pools
(ints -1..7, strings '', a, b, ab, A, ba, doubles k/4 and nan, NULL 15 %). Fills: VALUES (0-10 rows) or
`INSERT ... SELECT <expr of i> FROM range(n)` with n in {5, 50, 300, 1500, 3000} (multi-vector paths),
optional index, DELETE, UPDATE. Queries: CTEs (materialized hints), set operations, 1-3 FROM items with
INNER/LEFT/RIGHT/FULL/CROSS/comma/SEMI/ANTI joins and derived tables, WHERE, GROUP BY/HAVING with
aggregates (FILTER, DISTINCT), window functions (frames), scalar/EXISTS/IN/ANY/ALL subqueries (correlated),
DISTINCT, ORDER BY/LIMIT/OFFSET. Typed by construction; ~5 % of cases error on both sides.
Work bound: the product of the sizes of all table references in a case is at most 1e6 (else an inline
VALUES table is used), so unoptimized plans stay fast.

Nested/temporal surface (15 % of inner expression nodes, `gen_nested_expr`): list and struct literals,
lambdas (list_transform/list_filter), list/struct/row comparisons, list_contains/list_has_any, string_split,
regexp, DATE arithmetic with date_part/strftime/dayname, DECIMAL casts, printf; aggregates
`list_sort(list(DISTINCT x))`, quantile_disc, median, min/max over a STRUCT. All return one of the four column
types, so they combine with every other construct. Avoided because the binder rejects them (both sides error):
subqueries inside lambda bodies, and subqueries as arguments of min/max over STRUCT ("Serialization Error: Cannot
copy BoundSubqueryExpression", e.g. `SELECT max(struct_pack(a := (SELECT 1), b := 'x')).b FROM t`; a
limitation worth reporting separately).

## Known differences masked (each counted in stats.json)

- `err_mismatch`: one side errors (non-INTERNAL), the other succeeds. Different plans may evaluate a failing
  cast/arithmetic on rows the other plan filters out (test/configs/verify_statement_explain_sql.json skip
  reasons). Not compared.
- `both_err`: both error (messages not compared); `err_kinds` lists the classes.
- `variant_timeout`: the variant plan exceeded max(5 s, 10x the default plan); no comparison.
- Determinism by construction instead of masks: no LIMIT/row_number/lag/lead/first_value/ROWS frames
  without an ORDER BY over all row columns; no float rounding (DOUBLE values are multiples of 1/4, no
  float division/multiplication); no -0.0 sources; no order-sensitive aggregates (string_agg, list, first).

## Throughput

~33-41 cases/s per process with 4 processes running on an otherwise idle machine; ~20-25 with other load (2 DuckDB threads per worker); 13-16 while a DuckDB
build runs alongside. Per case ~8 ms setup (DROP/CREATE/INSERT), ~10 ms per query execution.

## Debug knobs

`DH_SHOW_ERR=<substring>` prints default-query errors containing it; `DH_SLOW_MS=<ms>` prints slow
default queries; `duckharness sample <seed> <n>` prints generated cases.

## Mutants (`mutants/*.diff`)

- `stats-ge-boundary`: statistics propagation prunes `x >= c` as always false when `c == max(x)`.
- `outer-join-wrong-side`: outer join simplification marks the wrong side as non-preserving.
- `topn-drops-offset`: ORDER BY + LIMIT + OFFSET -> TopN ignores the OFFSET.
- `or-null-nonroot`: `x OR NULL` simplified to `x` in non-root filter positions.

Own gate run (4 x 240 s per mutant; caught = a finding whose repro.sh exits 1 on the mutant and 0 on base):
topn-drops-offset 12 findings (`optdiff__rows_top_n`), stats-ge-boundary 4 (`..._statistics_propagation`,
`..._join_order`), outer-join-wrong-side 3 (`..._outer_join_simplification` after localization), or-null-nonroot 4 (`..._expression_rewriter`). Base: failure rate ~0.1 %, all real-looking
(see below).

## Base findings seen in the first runs (unreviewed; triage pending)

- `crash__assert_plan_comparison_join` (`op.left_projection_map.empty() || op.join_type == JoinType::MARK`)
  and `crash__internal_vector_reference_used_on_vector`: FULL JOIN with a subquery in its ON clause followed
  by another join, under some disabled optimizer / enable_optimizer=false.
- `optdiff__rows_filter_pushdown`: with `disabled_optimizers='filter_pushdown'`,
  `SELECT nullif((SELECT '' FROM (SELECT 0.0 FROM t1) x4), '') FROM t1 x1 WHERE CAST(NULL AS BOOLEAN)` returns a row.
- `crash__assert_propagate_get`, `crash__assert_physical_nested_loop_join`, a non-reproducing `crash__signal`
  (worker SIGSEGV in a variant query).
