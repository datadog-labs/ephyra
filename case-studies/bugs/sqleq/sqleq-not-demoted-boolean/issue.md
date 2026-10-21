# Issue for datadog-labs/sqleq: filed as #186

Filed 2026-10-14 20:11 UTC by alpaylan: https://github.com/datadog-labs/sqleq/issues/186

Work tag `sqleq/bug/not-demoted-boolean`. Verified 2026-10-14 on sqleq main `c23db4d` with
qed-prover main 9e9c262: the minimal pair panics at ast.rs:630:1. Retyping the `q_bool_*` nodes
BOOLEAN in the emitted JSON makes QED prove the pair (and leaves the mutant NotProvable); the
source fix itself was not built. Found by ephyra-sqleq (seed 90, `is-false-not` variant). Full
triage: `triage.md`. Related: #174 (same panic site, different cause); the set-operation typing
draft in `../sqleq-ill-typed-in-setop/issue.md` (also VARBINARY defaults).

---

**Title:** frontend: `NOT` over a demoted boolean operator (`<@`, `@>`, `&&`, `~`, `!~`) outside a WHERE reaches the prover applied to a VARBINARY term (QED panics)

A `NOT` whose operand is one of the operators `demote_operators` turns into a `q_bool_*` call is emitted with that call typed `VARBINARY`, unless the `NOT` sits directly in a WHERE/ON/HAVING condition. The QED prover then panics building the `NOT`.

```sql
-- truth: equivalent
CREATE TABLE t (a text);
SELECT * FROM t WHERE (a ~ 'y') IS FALSE;
SELECT * FROM t WHERE (a !~ 'y') IS TRUE;
```

On `main` (`c23db4d`), the frontend emits, and qed-prover main (`9e9c262`) panics in `pipeline::null::Ctx::bool_not`:

```
thread 'main' panicked at .../z3-0.11.2/src/ast.rs:630:1:
assertion failed: !ast.is_null()
```

Side A's plan has `IS TRUE(NOT(Q_BOOL_REMATCH : BOOLEAN))`. Side B's has `IS TRUE(NOT(Q_BOOL_REMATCH : VARBINARY))`. The same happens with `(NOT (x <@ y)) IS TRUE` (`Q_BOOL_CONTAINS`), `NOT (x && y)` (`Q_BOOL_OVERLAP`) and with `SELECT a !~ 'y' FROM t` in a select list. PostgreSQL returns the same rows for the pair above, and also for `(ARRAY[a] <@ ARRAY[b]) IS FALSE` vs `(NOT (ARRAY[a] <@ ARRAY[b])) IS TRUE` over `int8` columns.

**Cause.** A demoted call is lowered by the generic call path. It has no declaration, so it gets `UNDECLARED_RET` (`VARBINARY`, `lower.rs:395`), and it becomes BOOLEAN only where `coerce_bool` runs. `coerce_bool` runs for AND/OR operands (`lower.rs:3293`), for `IS [NOT] TRUE/FALSE` operands (`lower.rs:3325`) and in `lower_bool`, but not for `NOT` in `lower_expr`: `unary` (`lower.rs:3189`) is `UnaryOperator::Not => Ok(not_bool(inner))`. So `x IS FALSE`, which lowers as `NOT(coerce_bool(x)) IS TRUE`, is well-sorted, while the spelled-out `(NOT x) IS TRUE` is not. `verify::check_levels` checks column references only, so nothing catches the ill-sorted `NOT`.

**Expected:** the `NOT`'s operand is BOOLEAN. Either `coerce_bool` it in `unary`'s `Not` arm, as the AND/OR arm does, or type `q_bool_*` calls BOOLEAN when they are lowered (`builtin_rtype` already says so). Setting those nodes to BOOLEAN in the emitted JSON makes QED prove the pair above and the original fuzzer pair. The first fix would also stop the panic in #174, which reaches the same `bool_not` through `NOT` over an opaque `COALESCE` (#174's typing of `$k` is a separate bug).

Found by ephyra-sqleq (seed 90, `is-false-not` variant).
