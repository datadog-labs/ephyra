**Title:** `x > ALL (subquery)` in the SELECT list fails with an internal schema error unless it is aliased

### Describe the bug

A quantified comparison such as `x > ALL (SELECT ...)` or `x = ANY (SELECT ...)` fails to plan when it appears in the SELECT list without an alias. The same query works when the expression is aliased (`AS r`) and when it is used in `WHERE`. Using it inside an aggregate, for example `bool_or(x > ALL (...))`, fails with `Schema error: No field named "bool_or(t.x > ALL)"`.

The error output itself says "This issue was likely caused by a bug in DataFusion's code."

### To Reproduce

```sql
CREATE TABLE t(x INT) AS VALUES (1), (6);
CREATE TABLE s(y INT) AS VALUES (5);
SELECT x > ALL (SELECT y FROM s) FROM t;
-- duckdb 1.5.6: false, true
-- datafusion main: Internal error (below)
SELECT bool_or(x > ALL (SELECT y FROM s)) FROM t;
-- duckdb 1.5.6: true
-- datafusion main: Schema error: No field named "bool_or(t.x > ALL)"
```

Actual output on `main` (datafusion-cli; schema dump shortened):

```
Error: Optimizer rule 'decorrelate_predicate_subquery' failed
caused by
Check optimizer-specific invariants after optimizer rule: decorrelate_predicate_subquery
caused by
Internal error: Assertion failed: compatible: Failed due to a difference in schemas: original schema: DFSchema { inner: Schema { fields: [Field { name: "t.x > ALL", data_type: Boolean, nullable: true }], ... }, new schema: DFSchema { inner: Schema { fields: [Field { name: "CASE WHEN EXISTS THEN Boolean(false) WHEN EXISTS THEN Boolean(NULL) ELSE Boolean(true) END", data_type: Boolean, nullable: true }], ... }.
This issue was likely caused by a bug in DataFusion's code. Please help us to resolve this by filing a bug report in our issue tracker: https://github.com/apache/datafusion/issues

Error: Optimizer rule 'optimize_projections' failed
caused by
Schema error: No field named "bool_or(t.x > ALL)".
Valid fields are "bool_or(CASE WHEN EXISTS THEN Boolean(false) WHEN EXISTS THEN Boolean(NULL) ELSE Boolean(true) END)".
```

For comparison, these work on the same build:

```sql
SELECT x > ALL (SELECT y FROM s) AS r FROM t;   -- false, true
SELECT x FROM t WHERE x > ALL (SELECT y FROM s); -- 6
SELECT x IN (SELECT y FROM s) FROM t;            -- false, false
```

### Expected behavior

The first query should return `false`, `true` (as DuckDB 1.5.6 does), the same result as the aliased version. An optimizer rewrite should not change a plan's output schema.

### Additional context

- Tested on `main` at c0ec5127ac99 (2026-10-22). Latest release 55.2.0 also fails: there, both queries fail in `optimize_projections`.
- `= ANY` and `< ANY` fail the same way.
- Cause: `RewriteSetComparison::rewrite_plan` (`datafusion/optimizer/src/rewrite_set_comparison.rs`) replaces the `SetComparison` with a `CASE WHEN EXISTS ...` expression but does not keep the original expression name. Other rewriting rules use `NamePreserver` for this. The output field of the Projection/Aggregate is renamed, and a later rule finds the old name missing.
- Possibly related, but not the same bug: #25258 and #25276 (other set-comparison issues), #23022.
- With the name preserved, the first query returns `false`, `true`. The `bool_or(...)` query then fails with `Physical plan does not support logical expression Exists`. That is a separate limitation: `SELECT bool_or(exists(SELECT y FROM s)) FROM t` fails the same way on `main`.
- I have a patch with tests and can open a PR.
- Found with an automated differential/metamorphic testing tool; reproduced on current `main`.
