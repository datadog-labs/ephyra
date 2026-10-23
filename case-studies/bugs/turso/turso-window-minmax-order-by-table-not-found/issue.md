Filed as #9883 (https://github.com/tursodatabase/turso/issues/9883) (line 1 corrected 2026-10-21).

# "Parse error: table not found" for ORDER BY rowid, then a min/max window column with a non-deterministic window ORDER BY

## Reproduction

```sql
CREATE TABLE t0(c0);
INSERT INTO t0 VALUES(1),(2);
SELECT rowid, max(c0) OVER (ORDER BY strftime(NULL) ROWS CURRENT ROW) FROM t0 ORDER BY 1, 2;
```

Run with `tursodb -q -m list test.db < repro.sql`.

## Turso output

```
  × Parse error: table not found
```

## SQLite 3.50.4 output

```
1|1
2|2
```

(Trunk gives the same.)

## Expected

Same as SQLite. The query is valid; the error comes from an internal lookup.

## What is needed to trigger it

- The aggregate is `max` or `min` (`sum`, `avg`, `total`, `count`, `group_concat`, `row_number` work).
- The window has a frame clause (`ROWS CURRENT ROW`, `RANGE BETWEEN CURRENT ROW AND CURRENT ROW`, `ROWS BETWEEN 1 PRECEDING AND CURRENT ROW`). Without a frame it works.
- The window's ORDER BY or PARTITION BY is a non-deterministic function (`strftime(NULL)`, `random()`, `changes()`). With `c0`, `abs(c0)` or `1+1` it works.
- The outer ORDER BY starts with the rowid (`rowid`, or an `INTEGER PRIMARY KEY` column) and then names the window column (`ORDER BY 1, 2`, `ORDER BY rowid, 2 DESC`). `ORDER BY 1` alone, `ORDER BY 2` alone, or `ORDER BY c0, 2` on a table without an INTEGER PRIMARY KEY work.
- A named window (`WINDOW w AS (...)`) works.

## Where

The message is the `find_table_by_internal_id(..).ok_or_else(|| ParseError("table not found"))` in the collation lookup in core/translate/collate.rs. `min`/`max` and ORDER BY terms are the places that need a collation, so it looks like the collation of the window column (from `max`'s argument `c0`) is looked up against a set of table references that does not contain `t0`. I did not find the exact caller.

Tested on Turso main at f8549ca2 (2026-10-13).
