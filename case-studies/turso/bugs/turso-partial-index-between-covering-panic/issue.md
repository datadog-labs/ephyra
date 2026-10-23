Filed as #9926 (https://github.com/tursodatabase/turso/issues/9926) (line 1 corrected 2026-10-21).

Title: Panic "column N of table tN is not stored in the index and the table cursor is not open" with a partial index and a BETWEEN that implies its WHERE

```sql
CREATE TABLE t(a INT, b);
CREATE INDEX i ON t(b) WHERE a >= 2;
INSERT INTO t VALUES (3, 'x'), (1, 'y');
SELECT count(*) FROM t WHERE a BETWEEN 2 AND 4;
```

Turso (`tursodb -q -m list`):

```
thread 'main' panicked at core/translate/expr/translator.rs:2571:33:
column 0 of table t1 is not stored in the index and the table cursor is not open
```

SQLite 3.50.4 (trunk is the same):

```
1
```

`SELECT b FROM t WHERE a BETWEEN 2 AND 4` and `... WHERE a BETWEEN 2 AND NULL` panic the same way. Writing the range out as `a >= 2 AND a <= 4` works, and so does `SELECT a ...`, because there column `a` is needed anyway.

Expected: `1`, no panic.

Likely cause: `parse_where` (core/translate/planner.rs ~2469) binds the BETWEEN once, so `mark_column_used` counts `a` once in `column_use_counts`. Then `rewrite_between_exprs` clones the bound `a` into two terms, `a >= 2` and `a <= 4`. In `apply_expression_index_coverage` (core/translate/plan.rs ~3349), the `a >= 2` usage matches the partial index's WHERE clause (`matches_where_clause`), so it accounts for 1 use of `a`. That equals the recorded count, and `a` is dropped from the required columns. The planner then picks a covering scan of `i` with no table cursor, while `a <= 4` still has to read `a`. Same panic message and the same coverage function as #9577 (expression index `ON t(b=1)` with `WHERE b=1`), but a different trigger.

Tested on Turso main f8549ca2.
