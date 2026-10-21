NOT POSTED, retired 2026-10-21: on Turso main fe4beb8 none of these queries panics any more (the build input is still materialized); #9847 was closed as fixed on 2026-10-16.

The same panic also happens with a plain inner join, with no RIGHT/FULL JOIN, when Turso materializes the build side of a hash join ("MATERIALIZE hash build input" in EXPLAIN QUERY PLAN). The subquery can be in the select list, the WHERE clause, an ON clause or LIMIT, and it can be uncorrelated:

```sql
CREATE TABLE t0(c0);
CREATE TABLE t1(c0, c1);
CREATE INDEX t1_i ON t1(c1);
INSERT INTO t0 VALUES(1),(2);
INSERT INTO t1 VALUES(1,5),(2,6);
SELECT (SELECT 1 FROM json_each('[1]')) FROM t0 JOIN t1 ON t0.c0 = t1.c0 WHERE t1.c1 = 5;
```

Turso:
```
thread 'main' panicked at core/vdbe/builder.rs:1098:9:
duplicate cursor key
```
SQLite 3.50.4: `1`

These panic in the same way:
- `SELECT t0.c0 FROM t0 JOIN t1 ON t0.c0 = t1.c0 WHERE t1.c1 = 5 AND EXISTS (SELECT 1 FROM json_each('[1]'));`
- `SELECT t0.c0 FROM t0 JOIN t1 ON t0.c0 = t1.c0 AND (SELECT 1 FROM generate_series(1,2)) WHERE t1.c1 = 5;`
- with `t1(c0 INTEGER PRIMARY KEY)` and no index: `SELECT (SELECT count(*) FROM json_each(NULL)) FROM t1 JOIN t0 ON t0.c0 = t1.c0 WHERE t1.c0 IS NULL;`

Here the second emission comes from a different place. `build_materialized_build_input_plan` (core/translate/emitter/select.rs) copies into the materialization subplan every non-FROM subquery that reads only tables in the join prefix, and an uncorrelated subquery always passes that test. The subplan emits it, then the main query emits it again with the same table ids. For a B-tree table the second `open_cursors` reuses the cursor; for `Table::Virtual` `alloc_cursor_id_keyed` asserts. EXPLAIN on the same query with a B-tree table in the subquery (`SELECT (SELECT 1 FROM t2) ...`) shows its code twice, once before `Insert ... hash_build_input_*` and once in the main query.

Backtrace (shortened): `alloc_cursor_id_keyed` <- `open_cursors` <- `emit_query` (subquery) <- `emit_non_from_clause_subquery` <- `emit_non_from_clause_subqueries_for_eval_at` <- `emit_query` <- `emit_program_for_select_with_inputs` (materialization subplan).

Tested on main at f8549ca2.
