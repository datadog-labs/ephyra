# Issue for datadog-labs/sqleq: filed as #177

Filed 2026-10-14 13:54 UTC by alpaylan: https://github.com/datadog-labs/sqleq/issues/177

Work tag `sqleq/bug/opaque-fn-in-setop`. Verified 2026-10-14 on sqleq main `c23db4d` with
qed-prover upstream main (`/nix/store/0fwjvgy83yk0q2bbfixnbs0p2cyp8hss-qed-prover-0.1.0`): 22
pairs in `verify/pairs`, results in `verify/out/summary.txt`. The IN-subquery half of the original
finding is already fixed on main by #78 (after our old checkout 97e6489); this is the set-operation
and VALUES half. Not a duplicate of #174 (different construct and code path). PostgreSQL 20devel
(158b6d5109) confirms the main pair is equivalent (both sides resolve to numeric, same rows) and the
VALUES pair is equivalent as values (`1` and `1.0` are equal numerics that print differently).

---

**Title:** frontend: a set operation or VALUES list whose branches have different types reaches the prover ill-typed when the first branch is REAL, BOOLEAN, DATE or opaque (QED panics)

A set operation (`UNION [ALL]`, `INTERSECT`, `EXCEPT`) whose branches give a column two different
types is emitted with those types as they are, unless the pair is one the set-operation check refuses.
That check only looks at temporal pairs and at equality classes (`temporal_mismatch || hides_coarse`).
It does not check that the sorts agree. So when the first branch is REAL, BOOLEAN, DATE or VARBINARY
and a later branch has another type, the plan has a column with one declared type that holds values of
another. The QED prover asserts that they match and panics. `VALUES` lists have the same check and the
same gap.

```sql
-- truth: equivalent
CREATE TABLE t (a numeric, i int4);
SELECT a FROM t UNION ALL SELECT i FROM t;
SELECT a FROM t UNION ALL SELECT i::numeric FROM t;
```

`sqleq-check --axes frontend,qed pair.sql` on `main` (`c23db4d`): frontend `emit`, qed **`panic`**:

```
thread 'main' panicked at src/pipeline/partial.rs:185:9:
assertion `left == right` failed: Wrong type for $1
  left: Real
 right: Integer
```

The plan for query A is
`{"union": [{"project": {... "target": [{"column": 0, "type": "REAL"}]}}, {"project": {... "target": [{"column": 2, "type": "INTEGER"}]}}]}`.
Postgres resolves the column to `numeric` and converts `i`, so A is B. Query B, with the cast written
out, lowers to the plan A should have had.

Which way round the branches are matters. With the branches swapped
(`SELECT i FROM t UNION ALL SELECT a FROM t`), the pair is refused (`unsupported: set operation over
columns of type INTEGER and REAL`), because `hides_coarse(INTEGER, REAL)` holds and
`hides_coarse(REAL, INTEGER)` does not. Other pairs that get through and panic in the same place
(qed-prover main, `partial.rs:185`):

| first branch | later branch | example | `left` / `right` |
|---|---|---|---|
| `numeric` column | `abs(a)` (undeclared, VARBINARY) | `SELECT a FROM t INTERSECT SELECT abs(a) FROM t` vs the same with `WHERE a IS NOT NULL` in the second branch (equivalent) | `Real` / `Custom("VARBINARY")` |
| `upper(a)` (VARBINARY) | `text` column | `SELECT upper(a) FROM t UNION ALL SELECT a FROM t` vs `... SELECT b FROM t` | `Custom("VARBINARY")` / `String` |
| `boolean` column | `starts_with(x, 'a')` (VARBINARY) | `SELECT p FROM t UNION ALL SELECT starts_with(x, 'a') FROM t` vs `SELECT q ...` | `Boolean` / `Custom("VARBINARY")` |
| `date` column | `to_date(x, 'YYYY')` (VARBINARY) | `SELECT d FROM t UNION ALL SELECT to_date(x, 'YYYY') FROM t` vs `SELECT e ...` | `Integer` / `Custom("VARBINARY")` |
| `numeric` column | `i + 1` (INTEGER) | `SELECT a FROM t UNION ALL SELECT i + 1 FROM t` vs `SELECT b ...` | `Real` / `Integer` |

`VALUES`: `SELECT x FROM (VALUES (1.5), (1)) AS v(x)` against `... (VALUES (1.5), (1.0)) ...`
(equivalent: Postgres resolves both to `numeric`, and `1 = 1.0`) is emitted with `"schema": ["REAL"]`
and an `INTEGER` literal in the second row. QED panics at
`src/pipeline/normal.rs:534:17: $1 and "1" have different types` (`left: Real`, `right: Integer`).

Controls: the same shapes with matching branch types (`SELECT a ... UNION ALL SELECT b ...`,
`upper(b) ... UNION ALL upper(a)`, `i::numeric` in place of `i`) are answered without a panic.
The `IN (subquery)` form of this mismatch (#67 §5) is fixed on `main` by #78:
`upper(a) IN (SELECT a ...)` and `i IN (SELECT numeric_col ...)` now come out well-typed and are
answered.

**Where it comes from**

- `lower_setexpr_ctx`, `src/lower.rs:1188-1199`: the only check on the branch column types is
  `temporal_mismatch(a, b) || hides_coarse(a, b)`, and then the operation takes `lcols`, the first
  branch's types. `hides_coarse` (`src/types.rs:1090`) is about equality classes (#108), not sorts.
  It is deliberately false for a REAL, BOOLEAN or temporal first branch, and for an opaque first branch
  over an identity type ("hides nothing"). That is right for its own purpose, but nothing else then
  makes the sorts agree.
- `lower_values`, `src/lower.rs:1223-1243`: the same check over the rows.
- Undeclared functions are VARBINARY (`UNDECLARED_RET`, `src/lower.rs:395`, used at `:442`), so any
  undeclared call in one branch against a column in the other is a mismatch.
- `coerce_cmp`'s doc (`src/types.rs:1171`) says numeric mismatches "are left for the prover's own
  promotion". QED has none in a set operation: `Relation::scope()` for `union`/`intersect`/`except` is
  the first branch's scope, and the column read above the operation asserts that type
  (`partial.rs:185`). This is also filed upstream for QED's own parser as qed-solver/prover#14.
- `verify::check_levels` (`src/verify.rs:133`, `:234`, run on every lowering from `src/lib.rs:424`)
  mirrors the prover by taking the first branch's scope and checks each branch on its own. It never
  compares branch types with each other, or `VALUES` rows with the schema, so this IR passes it.

**Suggested fix**

Do what #78 did for `IN`: give each output column one type across the branches (or rows). Where
Postgres's resolution is a conversion `cast_to` / `cast_into` already makes for `=` (numeric promotion,
an opaque common type named as `x = c` names it), wrap the branch that needs it in a projection that
applies it. Otherwise refuse (`set operation over columns of type REAL and INTEGER`), as the temporal
case is refused now. Keep the `hides_coarse` refusal as it is. Separately, have `check_levels` check
that every set-operation branch has the first branch's types and that every `VALUES` row matches
`schema`, so any remaining mismatch is a frontend error and not a prover panic.

**Related**

- #67 §5 / #78: the same panic class through `IN (subquery)`, fixed there. This is the set-operation
  and `VALUES` half, which the fix did not cover.
- #174: another opaque-typed term reaching QED ill-sorted (`NOT` over a `COALESCE` with `$k`). Same
  class of symptom, but a different construct and code path.
- qed-solver/prover#14: QED itself neither promotes nor rejects mismatched `IN`/`UNION` operands,
  and its own parser emits them for INTEGER vs DECIMAL.

Environment: sqleq `c23db4d` (release build of `sqleq-frontend` and `sqleq-check`), macOS arm64;
qed-prover built from upstream main
(`/nix/store/0fwjvgy83yk0q2bbfixnbs0p2cyp8hss-qed-prover-0.1.0`).
