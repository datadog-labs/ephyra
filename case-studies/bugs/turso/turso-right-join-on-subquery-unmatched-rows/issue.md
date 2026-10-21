DRAFT, not filed

# RIGHT JOIN followed by LEFT JOIN: a subquery in the later ON clause is not re-evaluated for unmatched right rows (panic or wrong result)

## Reproduction 1: panic

```sql
CREATE TABLE t0(c0);
CREATE TABLE t1(c0);
CREATE TABLE t3(c0);
INSERT INTO t0 VALUES(1);
INSERT INTO t3 VALUES(1);
SELECT * FROM t1 RIGHT JOIN t0 ON 0 LEFT JOIN t3 ON 1 IN (SELECT t1.c0 FROM t3 AS b);
```

Turso:
```
thread 'main' panicked at core/vdbe/execute.rs:6820:42:
cursor id 0 is None
```
SQLite 3.50.4: `|1|`

## Reproduction 2: wrong result

```sql
CREATE TABLE t0(c0);
CREATE TABLE t1(c0);
CREATE TABLE t3(c0);
INSERT INTO t0 VALUES(1);
INSERT INTO t1 VALUES(2);
INSERT INTO t3 VALUES(1);
SELECT * FROM t1 RIGHT JOIN t0 ON 0 LEFT JOIN t3 ON EXISTS (SELECT 1 FROM t3 AS b WHERE t1.c0 IS NULL);
```

Turso: `|1|`
SQLite 3.50.4: `|1|1`

## Expected

Same as SQLite (trunk agrees with 3.50.4). The row `(NULL, 1)` is an unmatched t0 row, so `t1.c0` is NULL. The ON condition of the LEFT JOIN must be evaluated with that NULL. In reproduction 2 it is true, so t3's row must be joined.

## What happens

The subquery in the LEFT JOIN's ON clause reads only `t1`, so Turso evaluates it once per `t1` row, inside t1's loop. (EXPLAIN shows the IN list built with `OpenEphemeral` / `IdxInsert` right after `Rewind t1`.) After the main loop, the unmatched-right-rows pass sets `t1` to a NULL row (`NullRow`) and jumps (`Gosub`) into the body that holds the LEFT JOIN with t3. It does not evaluate the subquery again. So the condition uses:
- nothing, if t1 is empty: the ephemeral cursor was never opened, which gives the panic above (`cursor id N is None`, also seen at core/vdbe/mod.rs:1499), or an unset result register (EXISTS, so wrong result);
- the value computed for the last real t1 row, if t1 has rows: a stale value, so wrong result.

## Likely cause

`emit_unmatched_right_rows` (core/translate/main_loop/close.rs) copies and re-emits only the WHERE-clause subqueries of the right table (`unmatched_right_rows_plan.subqueries`). Subqueries that later tables' join conditions depend on, and that were placed at the loop of a table to the left of the RIGHT JOIN, are not emitted again for the NULL-extended row.

Tested on Turso main at 7641fad2 (2026-10-13).
