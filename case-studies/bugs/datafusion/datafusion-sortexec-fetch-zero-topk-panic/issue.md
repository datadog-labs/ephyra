**Title:** `ORDER BY ... LIMIT 0` panics in TopK (`assertion failed: k > 0`) when `eliminate_limit` does not run

### Describe the bug

A `SortExec` with `fetch = Some(0)` over unsorted input builds a `TopK` with `k = 0`, and `TopKHeap::new` panics on
`assert!(k > 0)` instead of returning an empty result.

With the default configuration, `eliminate_limit` turns `LIMIT 0` into an empty relation first, so this is not
reached. But it is reachable from plain SQL by setting `datafusion.optimizer.max_passes = 0`. The physical limit
pushdown then puts `fetch=0` on the sort. Embedders that remove or replace the logical optimizer rules, or build
`SortExec::with_fetch(Some(0))` directly, hit it too.

### To Reproduce

`datafusion-cli` built from `main` (63a2801c1b0b766cc206c2bbebde9c52dac66f60):

```sql
SET datafusion.optimizer.max_passes = 0;
CREATE TABLE t0 (c0 BIGINT) AS VALUES (3), (1), (2);
SELECT c0 FROM t0 ORDER BY c0 LIMIT 0;
```

```
thread 'main' panicked at datafusion/physical-plan/src/topk/mod.rs:946:9:
assertion failed: k > 0
```

`EXPLAIN` shows the plan is `SortExec: TopK(fetch=0)` over the scan.

### Expected behavior

An empty result (0 rows), as without the `SET`. `LIMIT 0` is valid, and the SQL reference says the count must be a
non-negative integer.

### Additional context

In `SortExec::execute` (`datafusion/physical-plan/src/sorts/sort.rs`), the `(false, Some(fetch))` arm passes `fetch`
to `TopK::try_new` without checking for 0. The `(true, Some(0))` arm is fine, because it uses `LimitStream`.

Code references (at 63a2801c1):

- The panic: `TopKHeap::new` asserts `k > 0`,
  [`topk/mod.rs#L944-L946`](https://github.com/apache/datafusion/blob/63a2801c1b0b766cc206c2bbebde9c52dac66f60/datafusion/physical-plan/src/topk/mod.rs#L944-L946).
  It is called with the fetch as `k` from `TopK::try_new`,
  [`topk/mod.rs#L408`](https://github.com/apache/datafusion/blob/63a2801c1b0b766cc206c2bbebde9c52dac66f60/datafusion/physical-plan/src/topk/mod.rs#L408).
- The missing check: the `match (sort_satisfied, self.fetch)` in `SortExec::execute`,
  [`sorts/sort.rs#L1609-L1629`](https://github.com/apache/datafusion/blob/63a2801c1b0b766cc206c2bbebde9c52dac66f60/datafusion/physical-plan/src/sorts/sort.rs#L1609-L1629).
  `(true, Some(fetch))` uses `LimitStream` and handles 0. `(false, Some(fetch))`
  ([L1617-L1629](https://github.com/apache/datafusion/blob/63a2801c1b0b766cc206c2bbebde9c52dac66f60/datafusion/physical-plan/src/sorts/sort.rs#L1617-L1629))
  goes straight to `TopK::try_new`.
- Why default settings don't hit it: `EliminateLimit` rewrites a literal `LIMIT 0` to an `EmptyRelation`,
  [`eliminate_limit.rs#L64-L72`](https://github.com/apache/datafusion/blob/63a2801c1b0b766cc206c2bbebde9c52dac66f60/datafusion/optimizer/src/eliminate_limit.rs#L64-L72).
- Why `max_passes = 0` hits it: the logical optimizer loop runs `while i < max_passes`,
  [`optimizer.rs#L621`](https://github.com/apache/datafusion/blob/63a2801c1b0b766cc206c2bbebde9c52dac66f60/datafusion/optimizer/src/optimizer.rs#L621),
  so no logical rule runs
  ([config doc, `config.rs#L1841-L1844`](https://github.com/apache/datafusion/blob/63a2801c1b0b766cc206c2bbebde9c52dac66f60/datafusion/common/src/config.rs#L1841-L1844)).
  The physical `LimitPushdown` then folds the limit into the sort through `with_fetch`,
  [`limit_pushdown.rs#L276`](https://github.com/apache/datafusion/blob/63a2801c1b0b766cc206c2bbebde9c52dac66f60/datafusion/physical-optimizer/src/limit_pushdown.rs#L276),
  which gives `SortExec: TopK(fetch=0)`.
- The dynamic filter a fix must complete: `mark_topk_emitted`,
  [`topk/mod.rs#L327`](https://github.com/apache/datafusion/blob/63a2801c1b0b766cc206c2bbebde9c52dac66f60/datafusion/physical-plan/src/topk/mod.rs#L327).
- `LIMIT 0` is valid SQL:
  [`select.md#L900`](https://github.com/apache/datafusion/blob/63a2801c1b0b766cc206c2bbebde9c52dac66f60/docs/source/user-guide/sql/select.md?plain=1#L900).

This is the same failure as #24404 (`PartitionedTopK requires k > 0`), which #24405 fixed by skipping the WindowTopN rewrite when the fetch is 0. This one is in the plain `TopK` path.

A possible fix is a `(false, Some(0))` arm that returns an empty stream without polling the input. It must also mark
the TopK dynamic filter as complete, so a scan waiting on it does not hang. I have a patch with a unit test and can
open a PR.

(Found with an automated differential-testing tool; I reproduced it by hand on current `main`.)
