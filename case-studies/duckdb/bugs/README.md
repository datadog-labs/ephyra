# Bugs: duckdb

The bugs the case studies found in duckdb. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [Wrong result: float/double `abs()` statistics are backwards when the column's max is 0 and its min is negative](duckdb-abs-double-stats-inverted-at-zero-bound/) | open | [link](https://github.com/duckdb/duckdb/issues/27711) |
| [Hang: correlated NOT EXISTS (... EXCEPT ALL ...) query never finishes on v2.0.0 (works on v1.5.6)](duckdb-buffered-cte-consumer-cycle-undetected-d/) | open | [link](https://github.com/duckdb/duckdb/issues/27737) |
| [EXPLAIN (SQL) exports quantile_disc with a column reference as the quantile argument when expression_rewriter is disabled](duckdb-cse-hoists-constant-quantile-argument/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27776) |
| [INTERNAL Error: assertion in join_hashtable.cpp:1474 for join query with WHERE NOT EXISTS (SELECT 1) (assertion builds)](duckdb-flush-caching-operators-continues-into-f/) | open | [link](https://github.com/duckdb/duckdb/issues/27722) |
| [INTERNAL Error "GetPartitioningSpaceRequirement: tuple width should not be 0" for a hash join whose probe side has no columns](duckdb-hash-join-zero-column-probe-side-partiti/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27450) |
| [Wrong result: WHERE (1 IN ((SELECT max(1) WHERE b), 1)) <> b drops a qualifying row (regression since v1.5.6)](duckdb-in-list-treated-as-null-propagating-sing/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27712) |
| [INTERNAL Error "Statistics mismatch: vector labeled as not having NULL values" with debug_verify_stats on a join with `a < b AND a = b - 1`](duckdb-join-condition-stats-narrowed-by-sibling/) | open | [link](https://github.com/duckdb/duckdb/issues/27725) |
| [INTERNAL Error in join elimination (`!column_bindings.empty()`) on a correlated subquery with a never-evaluated `count(DISTINCT ...)`](duckdb-join-elimination-asserts-on-conditionles/) | open | [link](https://github.com/duckdb/duckdb/issues/27445) |
| [INTERNAL Error "Projection map references a missing child column": join elimination leaves the parent filter's projection map stale](duckdb-join-elimination-stale-parent-projection/) | open | [link](https://github.com/duckdb/duckdb/issues/27449) |
| [Join order optimizer drops a filter that references no column (`WHERE NULL`, `WHERE random() > 2`) when filter_pushdown is disabled](duckdb-join-order-drops-relation-free-filter/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27451) |
| [INTERNAL Error: Assertion "normalized" in query_graph_manager.cpp on CROSS JOIN + LEFT JOIN ON (SELECT true) + WHERE](duckdb-join-order-filter-projection-map-hides-p/) | open | [link](https://github.com/duckdb/duckdb/issues/27723) |
| [INTERNAL Error "Failed to bind column reference" when an always-matching SEMI JOIN (`ON true`) is reordered by the join order optimizer](duckdb-join-order-stale-projection-map-after-se/) | open | [link](https://github.com/duckdb/duckdb/issues/27452) |
| [Wrong result: LIMIT 1 returns 3 rows when joined with an ungrouped aggregate over an empty table](duckdb-join-stats-clobbered-by-ungrouped-aggreg/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27714) |
| [INTERNAL Error "BoundReferenceExpression should not be used here yet!" for a lambda inside an aggregate inside a lambda](duckdb-lambda-in-aggregate-in-lambda-gets-outer/) | open | [link](https://github.com/duckdb/duckdb/issues/27720) |
| [INTERNAL Error !ContainsDependentJoin in planner.cpp: lateral reference in FULL JOIN ON subquery, join input with correlated subquery](duckdb-lateral-dependent-join-hoisted-cte-defin/) | open | [link](https://github.com/duckdb/duckdb/issues/27716) |
| [INTERNAL Error "Failed to bind column reference" for LEFT JOIN ... ON false AND x IN (SELECT ...) WHERE true](duckdb-left-join-unsatisfiable-condition-dummy-/) | open | [link](https://github.com/duckdb/duckdb/issues/27719) |
| [Segfault in `dense_rank() OVER ()` over a RIGHT_ANTI hash join that returns its unmatched rows](duckdb-natural-sort-empty-local-collection-dens/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27448) |
| [Nested loop / piecewise merge joins ignore the left projection map above an outer join with a subquery in its ON clause (INTERNAL errors, wrong columns)](duckdb-nested-loop-join-ignores-projection-map-/) | open | [link](https://github.com/duckdb/duckdb/issues/27717) |
| [Nested loop / piecewise merge joins ignore the left projection map above an outer join with a subquery in its ON clause (INTERNAL errors, wrong columns)](duckdb-nlj-ignores-projection-map-after-join-su/) | open | [link](https://github.com/duckdb/duckdb/issues/27717) |
| [INTERNAL Error "Vector::Reference used on vector of different type" for FULL JOIN with a subquery in ON over a CROSS JOIN, projection_pullup disabled](duckdb-pair-dependent-full-join-cte-side-column/) | open | [link](https://github.com/duckdb/duckdb/issues/27726) |
| [Nested loop / piecewise merge joins ignore the left projection map above an outer join with a subquery in its ON clause (INTERNAL errors, wrong columns)](duckdb-pwmj-ignores-projection-map-after-join-s/) | open | [link](https://github.com/duckdb/duckdb/issues/27717) |
| [INTERNAL Error (assertion string_stats.cpp:715) for SIMILAR TO filter above FULL JOIN](duckdb-regex-range-blob-bounds-assert-in-string/) | open | [link](https://github.com/duckdb/duckdb/issues/27724) |
| [SEMI JOIN with a right-side-only condition returns no rows when filter_pushdown is disabled](duckdb-semi-join-right-only-condition-no-filter-pushdown/) | open | [link](https://github.com/duckdb/duckdb/issues/27633) |
| [INTERNAL Error `exprs.size() > 0` (expression_executor.cpp) when statistics propagation folds an aggregate with no aggregate expressions (`HAVING` without aggregates)](duckdb-stats-folds-expressionless-aggregate-int/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27446) |
| [INTERNAL Error `physical_index != DConstants::INVALID_INDEX` in propagate_get.cpp when statistics fold a pushed-down filter's column away](duckdb-stats-table-filter-loses-column-ref/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27447) |
| [Wrong result: WHERE contains(CASE WHEN b THEN 'a' END, 'a') keeps rows where the predicate is NULL](duckdb-table-filter-true-or-null-becomes-column/) | open | [link](https://github.com/duckdb/duckdb/issues/27715) |
