# Bugs: duckdb

The bugs the case studies found in duckdb. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [duckdb-abs-double-stats-inverted-at-zero-bound](duckdb-abs-double-stats-inverted-at-zero-bound/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27711) |
| [duckdb-buffered-cte-consumer-cycle-undetected-d](duckdb-buffered-cte-consumer-cycle-undetected-d/) | reported 2026-10-23 | [link](https://github.com/duckdb/duckdb/issues/27737) |
| [EXPLAIN (SQL) exports quantile_disc with a column reference as the quantile argument when expression_rewriter is disabled](duckdb-cse-hoists-constant-quantile-argument/) | open | [link](https://github.com/duckdb/duckdb/issues/27776) |
| [duckdb-flush-caching-operators-continues-into-f](duckdb-flush-caching-operators-continues-into-f/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27722) |
| [INTERNAL Error "GetPartitioningSpaceRequirement: tuple width should not be 0" for a hash join whose probe side has no columns](duckdb-hash-join-zero-column-probe-side-partiti/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27450) |
| [duckdb-in-list-treated-as-null-propagating-sing](duckdb-in-list-treated-as-null-propagating-sing/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27712) |
| [duckdb-join-condition-stats-narrowed-by-sibling](duckdb-join-condition-stats-narrowed-by-sibling/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27725) |
| [INTERNAL Error in join elimination (`!column_bindings.empty()`) on a correlated subquery with a never-evaluated `count(DISTINCT ...)`](duckdb-join-elimination-asserts-on-conditionles/) | open | [link](https://github.com/duckdb/duckdb/issues/27445) |
| [INTERNAL Error "Projection map references a missing child column": join elimination leaves the parent filter's projection map stale](duckdb-join-elimination-stale-parent-projection/) | open | [link](https://github.com/duckdb/duckdb/issues/27449) |
| [Join order optimizer drops a filter that references no column (`WHERE NULL`, `WHERE random() > 2`) when filter_pushdown is disabled](duckdb-join-order-drops-relation-free-filter/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27451) |
| [duckdb-join-order-filter-projection-map-hides-p](duckdb-join-order-filter-projection-map-hides-p/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27723) |
| [INTERNAL Error "Failed to bind column reference" when an always-matching SEMI JOIN (`ON true`) is reordered by the join order optimizer](duckdb-join-order-stale-projection-map-after-se/) | open | [link](https://github.com/duckdb/duckdb/issues/27452) |
| [duckdb-join-stats-clobbered-by-ungrouped-aggreg](duckdb-join-stats-clobbered-by-ungrouped-aggreg/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27714) |
| [duckdb-lambda-in-aggregate-in-lambda-gets-outer](duckdb-lambda-in-aggregate-in-lambda-gets-outer/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27720) |
| [duckdb-lateral-dependent-join-hoisted-cte-defin](duckdb-lateral-dependent-join-hoisted-cte-defin/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27716) |
| [duckdb-left-join-unsatisfiable-condition-dummy-](duckdb-left-join-unsatisfiable-condition-dummy-/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27719) |
| [Segfault in `dense_rank() OVER ()` over a RIGHT_ANTI hash join that returns its unmatched rows](duckdb-natural-sort-empty-local-collection-dens/) | open | [link](https://github.com/duckdb/duckdb/issues/27448) |
| [duckdb-nested-loop-join-ignores-projection-map-](duckdb-nested-loop-join-ignores-projection-map-/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27717) |
| [duckdb-nlj-ignores-projection-map-after-join-su](duckdb-nlj-ignores-projection-map-after-join-su/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27717) |
| [duckdb-pair-dependent-full-join-cte-side-column](duckdb-pair-dependent-full-join-cte-side-column/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27726) |
| [duckdb-pwmj-ignores-projection-map-after-join-s](duckdb-pwmj-ignores-projection-map-after-join-s/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27717) |
| [duckdb-regex-range-blob-bounds-assert-in-string](duckdb-regex-range-blob-bounds-assert-in-string/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27724) |
| [duckdb-semi-join-right-only-condition-no-filter-pushdown](duckdb-semi-join-right-only-condition-no-filter-pushdown/) | reported 2026-10-21 | [link](https://github.com/duckdb/duckdb/issues/27633) |
| [INTERNAL Error `exprs.size() > 0` (expression_executor.cpp) when statistics propagation folds an aggregate with no aggregate expressions (`HAVING` without aggregates)](duckdb-stats-folds-expressionless-aggregate-int/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27446) |
| [INTERNAL Error `physical_index != DConstants::INVALID_INDEX` in propagate_get.cpp when statistics fold a pushed-down filter's column away](duckdb-stats-table-filter-loses-column-ref/) | open | [link](https://github.com/duckdb/duckdb/issues/27447) |
| [duckdb-table-filter-true-or-null-becomes-column](duckdb-table-filter-true-or-null-becomes-column/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27715) |
