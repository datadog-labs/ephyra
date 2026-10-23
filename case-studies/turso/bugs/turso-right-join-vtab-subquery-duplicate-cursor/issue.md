Filed as #9847 (https://github.com/tursodatabase/turso/issues/9847) (line 1 corrected 2026-10-21).

# Panic "duplicate cursor key" when a RIGHT/FULL JOIN query has a WHERE subquery over a table-valued function

## Reproduction

```sql
CREATE TABLE t0(c0);
CREATE TABLE t1(c0);
SELECT * FROM t0 RIGHT JOIN t1 ON 1 WHERE EXISTS (SELECT 1 FROM json_each('[1]'));
```

Run with `tursodb -q -m list test.db < repro.sql`.

## Turso output

```
thread 'main' panicked at core/vdbe/builder.rs:1098:9:
duplicate cursor key
```

## SQLite 3.50.4 output

No rows, no error. (SQLite trunk gives the same.)

## Expected

Same as SQLite: an empty result.

## Notes

- It happens with RIGHT JOIN and FULL JOIN, but not with LEFT JOIN or an inner join.
- It happens with any table-valued function in the WHERE subquery (`json_each`, `json_tree`, `generate_series`), whether or not the subquery is correlated. It also happens with `IN (SELECT value FROM json_each(...))` and `(SELECT count(DISTINCT value) FROM json_each(...))`. With a regular table in the subquery the query works.
- The panic happens while the statement is compiled, so the table contents do not matter.

## Likely cause

`emit_unmatched_right_rows` (core/translate/main_loop/close.rs) emits the WHERE-clause subqueries a second time, for the pass that outputs unmatched right rows (`emit_unmatched_rows_subquery`). The copied subquery plan keeps the table reference ids of the first copy. For a B-tree table, `JoinedTable::open_cursors` (core/translate/plan.rs) calls `alloc_cursor_id_keyed_if_not_exists`, so the second emit reuses the cursor. For `Table::Virtual` it calls `alloc_cursor_id_keyed`, which asserts that the key is new. That assert panics in `ProgramBuilder::alloc_cursor_id_keyed` (core/vdbe/builder.rs).

Backtrace (shortened): `alloc_cursor_id_keyed` <- ... <- `emit_non_from_clause_subquery` <- `with_cursor_overrides` <- `close::emit_unmatched_rows_subquery`.

Related, but a different code path: #4576 (closed), "duplicate cursor key" for a join of B-tree tables with `generate_series`.

Tested on Turso main at 7641fad2 (2026-10-13). Commits after that up to f8549ca2 change only `postgres/`.
