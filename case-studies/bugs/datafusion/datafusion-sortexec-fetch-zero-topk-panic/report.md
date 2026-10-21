# SortExec with `fetch = 0` panics in `TopKHeap::new` ("assertion failed: k > 0") instead of returning no rows

> **Owner notes (not part of the issue text)**
> - Reproduces on current `main`, 1da3cf049fa1a7a708acc2c29eba4f2e9dece8db (2026-10-16). Not fixed upstream.
> - **Duplicate search was not done.** The search helper was blocked. Check **#20448** first: intake's
>   known-issues list files it under "Panics (TopK)". See `dup-search.md`.
> - Low severity. Default-config SQL does not reach it, because the `eliminate_limit` rule turns `LIMIT 0` into an
>   empty relation first. It is reachable when an embedder removes that rule or builds a physical plan with
>   `SortExec::with_fetch(Some(0))`. SECURITY.md classes it as an ordinary bug.
> - The project accepts patches, but its AI policy wants a human to review any PR. `fix.diff` is included.

---

### Describe the bug

If a `SortExec` gets `fetch = Some(0)` and its input is not already sorted, `SortExec::execute` builds a `TopK`
with `k = 0`. `TopKHeap::new` then hits `assert!(k > 0)` and panics. The process aborts instead of returning an
empty result.

With the default optimizer rules, SQL like `ORDER BY ... LIMIT 0` never gets here, because `eliminate_limit`
rewrites `LIMIT 0` into an `EmptyRelation`. Other paths do produce the plan:

- an application removes or replaces the logical rules (`SessionContext::remove_optimizer_rule("eliminate_limit")`,
  `SessionStateBuilder::with_optimizer_rules(...)`);
- code builds or deserialises a physical plan containing `SortExec::with_fetch(Some(0))`.

`TopK(fetch=0)` is a plan the planner itself produces and prints (see the EXPLAIN output below), so the operator
should be able to run it.

### To Reproduce

A small Rust binary against `main` (needs `datafusion` and `tokio`):

```rust
use datafusion::error::Result;
use datafusion::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let ctx = SessionContext::new();
    assert!(ctx.remove_optimizer_rule("eliminate_limit"));
    ctx.sql("CREATE TABLE t0(c0 BIGINT) AS VALUES (3), (1), (2)").await?;

    let df = ctx.sql("SELECT c0 FROM t0 ORDER BY c0 LIMIT 0").await?;
    println!("{}", df.clone().explain(false, false)?.to_string().await?);
    let batches = df.collect().await?;
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    println!("rows returned: {rows}");
    Ok(())
}
```

Actual output on `main` (1da3cf049):

```
| logical_plan  | Sort: t0.c0 ASC NULLS LAST, fetch=0                                                |
|               |   TableScan: t0 projection=[c0]                                                    |
| physical_plan | SortExec: TopK(fetch=0), expr=[c0@0 ASC NULLS LAST], preserve_partitioning=[false] |
|               |   DataSourceExec: partitions=1, partition_sizes=[1]                                |

thread 'main' panicked at datafusion/physical-plan/src/topk/mod.rs:946:9:
assertion failed: k > 0
```

### Expected behavior

The query returns zero rows (`rows returned: 0`). It should not panic.

- The SQL reference (`docs/source/user-guide/sql/select.md`, "LIMIT and OFFSET clauses") says: "The count
  expressions must be constant expressions that evaluate to non-negative integers or `NULL`". So `LIMIT 0` is
  valid, and `LIMIT 0` means no rows.
  <https://github.com/apache/datafusion/blob/main/docs/source/user-guide/sql/select.md#limit-and-offset-clauses>
- `SECURITY.md`: "In general, crashes, panics, hangs, and excessive resource consumption (memory, CPU, or disk) are
  treated as **bugs**". <https://github.com/apache/datafusion/blob/main/SECURITY.md>

### Additional context

**Root cause.** In `SortExec::execute` (`datafusion/physical-plan/src/sorts/sort.rs`, around line 1617), the match
on `(sort_satisfied, self.fetch)` sends every `(false, Some(fetch))` to `TopK::try_new(..., *fetch, ...)`. Nothing
checks for zero. `TopKHeap::new(k)` (`datafusion/physical-plan/src/topk/mod.rs:946`) asserts `k > 0`. The
`(true, Some(0))` arm is fine, because it uses `LimitStream`. The assertion appears to date from when the TopK operator was
added (1cf808d0e, #7721, first released in 32.0.0), so earlier releases are probably affected too. I only ran
`main`.

**Suggested fix.** Add a `(false, Some(0))` arm before the general TopK arm. It returns an
`EmptyRecordBatchStream` for the output schema, so the input is never polled. The arm must also mark the TopK dynamic
filter as emitted (`TopKDynamicFilters::mark_topk_emitted`, made `pub(crate)`). Otherwise a consumer waiting for
the filter to complete (for example a scan using dynamic-filter pushdown) could wait forever. An alternative is to
return an error from `TopK::try_new` for `k == 0`, but an empty result is what the plan means.

The attached `fix.diff` does this and adds a unit test, `sorts::sort::tests::topk_fetch_zero`. The test checks that
no rows come back and that the dynamic filter completes. With the patch applied to `main`, the reproducer above
prints `rows returned: 0`. `cargo test -p datafusion-physical-plan --lib -- sorts:: topk::` passed (283 tests) on
the base commit 9f4b005e6 during triage. I did not rerun that suite on 1da3cf049.

**Version tested:** `main` at 1da3cf049fa1a7a708acc2c29eba4f2e9dece8db (workspace version 55.1.0), Linux x86_64,
stable Rust.
