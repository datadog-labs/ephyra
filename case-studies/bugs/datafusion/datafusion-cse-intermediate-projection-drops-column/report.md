**Title:** `GROUP BY CAST(a AS VARCHAR), a` with a common subexpression fails in `optimize_projections` ("No field named t.a", sometimes a panic)

### Describe the bug

A query that groups by both `CAST(a AS VARCHAR)` and `a`, and has a common subexpression such as `a + 1`, fails to plan:
`Optimizer rule 'optimize_projections' failed ... No field named t.a`.
A slightly different query panics in `optimize_projections` instead.
The query is valid. Postgres and DuckDB return one row.

### To Reproduce

```sql
CREATE TABLE t (a INT) AS VALUES (1);
SELECT a, a + 1, (a + 1) * 2 FROM t GROUP BY CAST(a AS VARCHAR), a;
```

```
Optimizer rule 'optimize_projections' failed
caused by
Schema error: No field named t.a.
Valid fields are __common_expr_1.
```

This variant panicked in about half of the runs (5 of 10, and 4 of 10 on a later check). The other runs returned the same schema error (it depends on HashMap iteration order):

```sql
CREATE TABLE t (a INT) AS VALUES (1);
SELECT a, a + 1, COALESCE(min('x'), CAST(a AS VARCHAR)) FROM t GROUP BY CAST(a AS VARCHAR), a;
```

```
thread 'main' panicked at datafusion/optimizer/src/optimize_projections/mod.rs:590:82:
called `Result::unwrap()` on an `Err` value: SchemaError(FieldNotFound { field: Column { relation: Some(Bare { table: "t" }), name: "a" }, valid_fields: [Column { relation: None, name: "__common_expr_1" }, Column { relation: None, name: "t.a" }, Column { relation: None, name: "min(Utf8(\"x\"))" }] }, Some(""))
```

### Expected behavior

The query returns one row:

```
SELECT a, a + 1, (a + 1) * 2 FROM t GROUP BY CAST(a AS VARCHAR), a;
# postgres 18.6: 1 | 2 | 4
# duckdb 1.5.6:  1 | 2 | 4
# datafusion main: Optimizer rule 'optimize_projections' failed ... No field named t.a.
```

Without the CAST group key (`GROUP BY a`), DataFusion returns `1 | 2 | 4`.

### Additional context

- Tested on `main` at c0ec5127ac99 (2026-10-22) with `datafusion-cli`, default settings.
- On 55.2.0 the same query fails earlier, in `eliminate_group_by_constant` ("Projections require unique expression names"). #26356 fixed that error, and the query now reaches this bug.
- Related, not duplicates: #26516 (same error text, UNION after `propagate_empty_relation`) and #26221 (`x` and `CAST(x AS ...)` share a name).
- Where: `build_common_expr_project_plan` in `common_subexpr_eliminate.rs` dedups input fields by flat name (`qualified_name(qualifier, name)`). The aggregate outputs an unqualified field named `t.a` (from the CAST) next to the qualified `t.a`. Both have the flat name `"t.a"`, so the qualified `t.a` is dropped from the intermediate projection while the outer projection still uses it.

I have a patch with tests and can open a PR.

Found with an automated differential/metamorphic testing tool; reproduced on current `main`.
