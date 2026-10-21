DRAFT, not filed

# Panic "a WITHOUT ROWID table has no secondary index to drive its loop" in a RIGHT/FULL JOIN on a SQLite-created file

## Reproduction

Turso rejects `CREATE INDEX` on a WITHOUT ROWID table, so create the file with SQLite:

```sh
sqlite3 w.db <<'SQL'
CREATE TABLE t0(c0);
CREATE TABLE t1(c0 PRIMARY KEY, c1) WITHOUT ROWID;
CREATE INDEX t1_i0 ON t1(c1);
SQL
echo 'SELECT count(*) FROM t0 FULL JOIN t1 ON NULL;' | tursodb -q -m list w.db
```

## Turso output

```
thread 'main' panicked at core/translate/main_loop/open.rs:736:9:
a WITHOUT ROWID table has no secondary index to drive its loop
```

The same panic happens with and without `--experimental-without-rowid`, and for `SELECT count(*) FROM t0 RIGHT JOIN t1 ON NULL;` and `SELECT NULL FROM t0 FULL JOIN t1 ON NULL;`. `LEFT JOIN` and a plain `SELECT count(*) FROM t1` work.

## SQLite 3.50.4 output

```
0
```

## Expected

`0`, or a clean error if this case is meant to be unsupported. It should never panic.

WITHOUT ROWID support is experimental (COMPAT.md), and Turso cannot create a secondary index on a WITHOUT ROWID table (#9233). But Turso opens and reads such files, and a panic should be an error.

## Likely cause

The read that outputs unmatched right rows (`unmatched_right_rows_plan`) uses the covering secondary index `t1_i0`. `emit_unmatched_right_rows` (core/translate/main_loop/close.rs:520) then passes that index cursor to `emit_right_join_key` (core/translate/main_loop/open.rs). For a WITHOUT ROWID table, that function asserts that no index cursor is given. The planner should not pick a secondary index here, or the key should be read from the index, since every secondary index of a WITHOUT ROWID table stores the primary key columns.

Backtrace: `emit_right_join_key` (open.rs:736) <- `emit_unmatched_right_rows` (close.rs:520) <- `close::emit` (close.rs:252) <- `emit_query`.

Related: #7273 (open; Turso reads SQLite-created WITHOUT ROWID secondary indexes as NULL rows).

Tested on Turso main at 7641fad2 (2026-10-13).
