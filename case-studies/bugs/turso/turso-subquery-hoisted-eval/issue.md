Filed as #9851 (https://github.com/tursodatabase/turso/issues/9851) (line 1 corrected 2026-10-21).

# An uncorrelated scalar subquery runs before the scan, so it raises errors SQLite never reaches

Turso evaluates an uncorrelated scalar subquery once, before the main loop begins. SQLite evaluates it the first time the expression is reached. When the subquery can fail (for example `sum()` overflowing), Turso fails statements that SQLite completes: the outer table is empty, the WHERE clause is constant false/NULL, or the subquery sits in a `CASE`/`coalesce` branch that is never taken.

## Reproduction (`tursodb -q -m list test.db < repro.sql`)

```sql
CREATE TABLE t(a);
INSERT INTO t VALUES (9223372036854775807), (1);
CREATE TABLE e(x);
SELECT 'q1', (SELECT sum(a) FROM t) FROM e;                  -- e is empty
SELECT 'q2', CASE WHEN 0 THEN (SELECT sum(a) FROM t) END;    -- branch never taken
SELECT 'q3', coalesce(1, (SELECT sum(a) FROM t));            -- never reached
SELECT 'q4', 1 FROM t WHERE NULL AND (SELECT sum(a) FROM t); -- WHERE is never true
```

SQLite 3.50.4 (trunk f69fd5f5 gives the same output):
```
q2|
q3|1
```

Turso:
```
Runtime error: integer overflow
Runtime error: integer overflow
Runtime error: integer overflow
Runtime error: integer overflow
```

Expected: `sum()` raises "integer overflow" only when the subquery actually runs (https://sqlite.org/lang_aggfunc.html#sumunc). In q1 and q4 no row reaches the expression. In q2 and q3 the subquery's branch is never taken: per https://sqlite.org/lang_expr.html, CASE evaluates only the selected branch and `coalesce()` short-circuits. SQLite returns the rows shown above.

## Likely cause

`EXPLAIN SELECT (SELECT sum(a) FROM t) FROM e` in Turso places the subquery subroutine (`Once` ... `AggFinal`) at the start of the program, ahead of `OpenRead`/`Rewind` on `e`. SQLite emits `BeginSubrtn`/`Once` inside the loop at the point where the expression is used. In Turso, uncorrelated non-FROM subqueries resolve to `EvalAt::BeforeLoop` (`NonFromClauseSubquery::get_eval_at`, core/translate/plan.rs ~4335, defaulting to `BeforeLoop`). `emit_non_from_clause_subqueries_for_phase` (core/translate/subquery.rs ~2149) then emits them before the loop under `Insn::Once`, wherever the expression appears.

A related variant with a different code path: a constant expression in the result list is moved into the init block by `ProgramBuilder::emit_constant_insns`. It therefore runs even when the table is empty:

```sql
CREATE TABLE e(x);
SELECT abs(-9223372036854775807 - 1) FROM e;  -- SQLite: no rows; Turso: integer overflow
```

SQLite 3.50.4 also factors this constant, but it codes it under `Once` inside the loop, so it never runs on an empty table.

Related but distinct: #9038, where AND/OR evaluate an operand that SQLite skips.

Tested on Turso main 7641fad2 against SQLite 3.50.4.
