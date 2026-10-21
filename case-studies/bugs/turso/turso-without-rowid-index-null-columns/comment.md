DRAFT, not filed — comment for #7273

On main f8549ca2 the issue's reproducer no longer returns all-NULL rows: columns declared in the index are now right, but every other column is still NULL, including primary-key columns that SQLite appends to the index record (`a||1`, `b||2`, `a||3` instead of `a|y|1`, `b|z|2`, `a|x|3`). A seek through the index does the same, and DISTINCT then merges rows that differ only in the NULL columns:

```sh
sqlite3 w.db <<'SQL'
CREATE TABLE t(a, b, c, PRIMARY KEY(c)) WITHOUT ROWID;
CREATE INDEX t_a ON t(a);
INSERT INTO t VALUES(1,'x',10),(1,'y',20);
SQL
tursodb -q -m list --experimental-without-rowid w.db <<'SQL'
SELECT * FROM t WHERE a = 1;
SELECT DISTINCT * FROM t WHERE a = 1;
SELECT count(DISTINCT c) FROM t WHERE a = 1;
SQL
```

Turso: `1||`, `1||`, then `1||` (one row), then `0`.
SQLite 3.50.4: `1|x|10`, `1|y|20`, then the same two rows, then `2`.

In `EXPLAIN`, the non-index columns are read with `Column` from the table cursor, which is opened but never positioned: `ProgramBuilder::emit_deferred_seek` (`core/vdbe/builder.rs`) returns without emitting anything when the index has no rowid, assuming the plan is covering, but the planner still picks this index for queries that read columns outside it. SQLite seeks the table by the primary key it takes from the index record.
