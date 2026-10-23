Filed as #10399 (https://github.com/tursodatabase/turso/issues/10399) on 2026-10-21.
Title: PRAGMA index_list / index_info / index_xinfo don't show the primary-key index of a WITHOUT ROWID table

```sql
CREATE TABLE w(x, PRIMARY KEY(x)) WITHOUT ROWID;
SELECT 'list', * FROM pragma_index_list('w');
SELECT 'info', * FROM pragma_index_info('sqlite_autoindex_w_1');
SELECT 'xinfo', * FROM pragma_index_xinfo('sqlite_autoindex_w_1');
```

Turso (`tursodb --experimental-without-rowid -q -m list :memory: < repro.sql`): no rows from any of the three.

SQLite 3.50.4 (trunk gives the same):
```
list|0|sqlite_autoindex_w_1|1|pk|0
info|0|0|x
xinfo|0|0|x|0|BINARY|1
```

Expected: SQLite reports the primary key of a WITHOUT ROWID table as an index named `sqlite_autoindex_<table>_1` with origin `pk`, and index_info/index_xinfo describe it. The result is the same when SQLite created the file. A rowid table with `x TEXT PRIMARY KEY` is listed correctly by Turso.

Likely cause: the `IndexList` arm in core/translate/pragma.rs only walks `schema.get_indices(table)`, and Turso keeps no index entry for a WITHOUT ROWID table's primary key (the table b-tree is keyed by it), so nothing is emitted.

Tested on Turso main db441331.
