**Title:** Panic "all window expressions shall have the same sort keys" when two window functions share `ORDER BY a + 1` (debug builds)

### Describe the bug

Two window functions with the same `ORDER BY <expression>` make the physical planner panic in debug builds, when
one of the window functions has an argument that gets constant-folded (`1 + 1`):

```
thread 'main' panicked at datafusion/core/src/physical_planner.rs:1116:21:
all window expressions shall have the same sort keys, as guaranteed by logical planning
```

`EXPLAIN` of the query panics too. Release builds skip the assertion.

### To Reproduce

Run in `datafusion-cli` built with debug assertions (for example `cargo run -p datafusion-cli`):

```sql
CREATE TABLE t (a BIGINT, b BIGINT) AS VALUES (1, 10), (2, 20);
SELECT sum(1 + 1) OVER (ORDER BY a + 1), sum(b) OVER (ORDER BY a + 1) FROM t;
```

```
# DuckDB 1.5.6: 2 | 10
#               4 | 30
# DataFusion (main, debug build):
thread 'main' panicked at datafusion/core/src/physical_planner.rs:1116:21:
all window expressions shall have the same sort keys, as guaranteed by logical planning
```

It also panics with `max(1 + 1)`, `count(1 + 1)` or a different second function, and when the two window functions
are in the other order. It does not panic with `max(2)` instead of `max(1 + 1)`. It does not panic with `INT`
columns either: there `a + 1` gets a cast, which changes the expression.

### Expected behavior

The query returns two rows, `2, 10` and `4, 30`, as DuckDB does. The logical planner put both window functions in
one `WindowAggr` with the same `ORDER BY`, so the physical planner should not hit its assertion.

### Additional context

- Tested on `main` at c0ec5127ac99 (2026-10-22), debug build: panics. Latest release 55.2.0 built in debug mode: panics at
  `physical_planner.rs:901` (same assertion). The 55.2.0 release binary (no debug assertions) returns the correct rows.
- Where in the code: `simplify_expressions` folds `1 + 1` and wraps that window expression in an alias to keep its
  name. Then `common_sub_expression_eliminate` replaces the shared `a + 1` with `__common_expr_1`. It adds an inner
  alias (`__common_expr_1 AS t.a + Int64(1)`) only in the window expression that has no outer alias. The
  `debug_assert!` in `DefaultPhysicalPlanner` compares the sort keys including aliases, so the two keys look
  different. With that assertion relaxed to ignore aliases, `EXPLAIN` shows the optimized node (frames trimmed):
  `WindowAggr: windowExpr=[[sum(Int64(2)) ORDER BY [__common_expr_1 ASC NULLS LAST] AS ..., sum(t.b) ORDER BY [__common_expr_1 AS t.a + Int64(1) ASC NULLS LAST] AS ...]]`
- Possibly related: #17770 (CSE fails in a window function, closed), #24750 (prevent CSE from extracting window
  functions, merged).

Found with an automated differential/metamorphic testing tool; reproduced on current `main`.
