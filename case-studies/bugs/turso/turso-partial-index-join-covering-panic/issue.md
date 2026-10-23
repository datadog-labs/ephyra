Filed as #9934 (https://github.com/tursodatabase/turso/issues/9934) (line 1 corrected 2026-10-21).
Title: Panic "column N of table tN is not stored in the index and the table cursor is not open" with a partial index, a seek on its key and a join on that key

```sql
CREATE TABLE t(a, b);
CREATE TABLE u(x);
CREATE INDEX i ON t(a) WHERE b > 1;
INSERT INTO t VALUES (2, 3);
INSERT INTO u VALUES (2);
SELECT count(*) FROM t JOIN u ON u.x = t.a WHERE a = 2 AND b > 1;
```

Turso (`tursodb -q -m list`):

```
thread 'main' panicked at core/translate/expr/translator.rs:2571:33:
column 1 of table t1 is not stored in the index and the table cursor is not open
```

SQLite 3.50.4 (trunk is the same):

```
1
```

`LEFT JOIN u ON u.x = t.a`, `ON u.x = t.a + 0` and `WHERE a > 0 AND b > 1` panic the same way, and so does `EXPLAIN QUERY PLAN`. No panic without the join (`FROM t WHERE a = 2 AND b > 1`), with a cross join (`FROM t, u WHERE a = 2 AND b > 1`), with a join condition that doesn't use `t.a` (`ON u.x = 7`), or with `SELECT b ...`, where `b` is needed anyway.

Expected: `1`, no panic.

Backtrace: `emit_early_conditions` -> `emit_where_term` -> `translate_condition_expr` -> ... -> `translate_column_expr`. The `b > 1` term is still emitted. But `apply_expression_index_coverage` (core/translate/plan.rs ~3333) has counted that term as covered, because it matches the partial index's WHERE clause, so `b` was dropped from the required columns and no table cursor was opened. Same panic and the same coverage function as #9577 (expression index key) and #9926 (BETWEEN), but here a join on the indexed column triggers it.

Tested on Turso main f8549ca2.
