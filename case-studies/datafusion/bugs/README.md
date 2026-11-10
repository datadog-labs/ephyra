# Bugs: datafusion

The bugs the case studies found in datafusion. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [Panic "all window expressions shall have the same sort keys" when two window functions share `ORDER BY a + 1` (debug builds)](datafusion-cse-inner-alias-breaks-window-sort-key-a/) | open | [link](https://github.com/apache/datafusion/issues/26620) |
| [`GROUP BY CAST(a AS VARCHAR), a` with a common subexpression fails in `optimize_projections` ("No field named t.a", sometimes a panic)](datafusion-cse-intermediate-projection-drops-column/) | open | [link](https://github.com/apache/datafusion/issues/26621) |
| [SanityCheckPlan error with sort-merge join when a filter says `a + b = a`](datafusion-equivalence-class-member-contains-own-me/) | open | [link](https://github.com/apache/datafusion/issues/26623) |
| [Wrong result: `count(*) OVER ()` sees only the LIMIT rows when ORDER BY ... LIMIT sits on top of another window (target_partitions > 1)](datafusion-order-preserving-variant-pushes-topk-fet/) | fixed | [link](https://github.com/apache/datafusion/issues/26617) |
| [SanityCheckPlan error with sort-merge join when a filter makes a sort or join key constant (constant lost in `EquivalenceGroup` projection)](datafusion-projection-drops-constant-of-indirectly-/) | fixed | [link](https://github.com/apache/datafusion/issues/26624) |
| [`push_down_filter` substitutes the wrong expression when `col` and `CAST(col AS ...)` share a schema name (HAVING and correlated IN give type errors)](datafusion-push-down-filter-aggregate-group-name-co/) | fixed | [link](https://github.com/apache/datafusion/issues/26619) |
| [`= ANY (subquery)` in a `JOIN ... ON` over an aliased table fails: "No field named x.a"](datafusion-push-down-filter-moves-correlated-subque/) | open | [link](https://github.com/apache/datafusion/issues/26573) |
| [`x > ALL (subquery)` in the SELECT list fails with an internal schema error unless it is aliased](datafusion-rewrite-set-comparison-renames-aggregate/) | open | [link](https://github.com/apache/datafusion/issues/26622) |
| [Regression: `ORDER BY col, (scalar subquery) LIMIT` over a `RIGHT JOIN` fails in `SanityCheckPlan`](datafusion-scalar-subquery-sort-key-lost-by-project/) | fixed | [link](https://github.com/apache/datafusion/issues/26572) |
| [`ORDER BY ... LIMIT 0` panics in TopK (`assertion failed: k > 0`) when `eliminate_limit` does not run](datafusion-sortexec-fetch-zero-topk-panic/) | fixed | [link](https://github.com/apache/datafusion/issues/26431) |
| [Wrong results: `ORDER BY <double>, ... LIMIT n` drops `-0.0` rows when 0.0 arrives in an earlier batch (TopK dynamic filter)](datafusion-topk-filter-drops-negative-zero/) | open | [link](https://github.com/apache/datafusion/issues/26618) |
| [SanityCheckPlan error for `UNION ALL ... ORDER BY` when each arm selects the same column twice](datafusion-union-output-ordering-drops-equivalent-s/) | open | [link](https://github.com/apache/datafusion/issues/26625) |
| [SanityCheckPlan error for two window functions with `target_partitions = 1` (`WindowAggExec` loses `can_repartition` when its children are replaced)](datafusion-window-agg-exec-with-new-children-forces/) | open | [link](https://github.com/apache/datafusion/issues/26626) |
