Filed as #9887 (https://github.com/tursodatabase/turso/issues/9887) (line 1 corrected 2026-10-21).

# HAVING with a predicate that is itself a GROUP BY term (`GROUP BY c, a IS NULL HAVING a IS NULL`) filters on the wrong value

Follow-up to #9084 (closed): its repros (`HAVING (a | 3) > 1` with `GROUP BY c, (a | 3)`) are fixed on main, but when the GROUP BY term is a comparison or `IS NULL`, and the HAVING clause repeats it, the HAVING clause still reads the table column after the scan has finished.

## Reproduction

```sql
CREATE TABLE t(a INTEGER PRIMARY KEY, c TEXT);
INSERT INTO t VALUES (1, 'y'), (2, 'xy'), (3, 'z');
SELECT 'q1', c FROM t GROUP BY c, a IS NULL HAVING a IS NULL;
SELECT 'q2', c FROM t GROUP BY c, a = 2 HAVING a = 2;
SELECT 'q3', c FROM t GROUP BY c, a > 1 HAVING a > 1;
SELECT 'q4', c FROM t GROUP BY c, a IN (2, 3) HAVING a IN (2, 3);
SELECT 'q5', c, a > 1 FROM t GROUP BY c, a > 1 HAVING a > 1;
```

Turso (`tursodb -q -m list`, main f8549ca2):
```
q1|xy
q1|y
q1|z
```

SQLite 3.50.4 (trunk f69fd5f5 gives the same):
```
q2|xy
q3|xy
q3|z
q4|xy
q4|z
q5|xy|1
q5|z|1
```

## Expected

The SQLite rows. `a IS NULL` is false for every group, so q1 returns nothing; q2-q5 keep the groups where the predicate is true. Turso evaluates `a` in the group output step from the table cursor, which is past the last row, so `a` is NULL there: every group passes `IS NULL` and none passes `=`, `>` or `IN`. Selecting the term (q5) does not help.

## Likely cause

The group value of `a IS NULL` is stored and cached as an expression, but the HAVING clause is compiled with `translate_condition_expr` (`core/translate/expr/condition.rs`). `translate_leaf_condition_expr` splits `IS NULL`, comparisons, `IN` etc. into their operands and translates the operand `a` directly, so the cached register for the whole term is never used, and `a` alone is not among the stored group columns. `EXPLAIN` of q1 shows `Column` on the table cursor followed by `NotNull` inside the group output subroutine.

Tested on Turso main f8549ca2.
