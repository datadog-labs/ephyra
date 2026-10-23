# Bugs: datafusion

The bugs the case studies found in datafusion. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [datafusion-cse-inner-alias-breaks-window-sort-key-a](datafusion-cse-inner-alias-breaks-window-sort-key-a/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26620) |
| [datafusion-cse-intermediate-projection-drops-column](datafusion-cse-intermediate-projection-drops-column/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26621) |
| [datafusion-equivalence-class-member-contains-own-me](datafusion-equivalence-class-member-contains-own-me/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26623) |
| [datafusion-order-preserving-variant-pushes-topk-fet](datafusion-order-preserving-variant-pushes-topk-fet/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26617) |
| [datafusion-projection-drops-constant-of-indirectly-](datafusion-projection-drops-constant-of-indirectly-/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26624) |
| [datafusion-push-down-filter-aggregate-group-name-co](datafusion-push-down-filter-aggregate-group-name-co/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26619) |
| [datafusion-push-down-filter-moves-correlated-subque](datafusion-push-down-filter-moves-correlated-subque/) | reported 2026-10-21 | [link](https://github.com/apache/datafusion/issues/26573) |
| [datafusion-rewrite-set-comparison-renames-aggregate](datafusion-rewrite-set-comparison-renames-aggregate/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26622) |
| [datafusion-scalar-subquery-sort-key-lost-by-project](datafusion-scalar-subquery-sort-key-lost-by-project/) | reported 2026-10-21 | [link](https://github.com/apache/datafusion/issues/26572) |
| [`ORDER BY ... LIMIT 0` panics in TopK (`assertion failed: k > 0`) when `eliminate_limit` does not run](datafusion-sortexec-fetch-zero-topk-panic/) | open | [link](https://github.com/apache/datafusion/issues/26431) |
| [datafusion-topk-filter-drops-negative-zero](datafusion-topk-filter-drops-negative-zero/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26618) |
| [datafusion-union-output-ordering-drops-equivalent-s](datafusion-union-output-ordering-drops-equivalent-s/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26625) |
| [datafusion-window-agg-exec-with-new-children-forces](datafusion-window-agg-exec-with-new-children-forces/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26626) |
