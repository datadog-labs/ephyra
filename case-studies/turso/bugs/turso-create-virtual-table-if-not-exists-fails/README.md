# CREATE VIRTUAL TABLE IF NOT EXISTS fails with 'near "IF": syntax error' and creates nothing

Reported by others as https://github.com/tursodatabase/turso/issues/11352
(2026-11-04); found independently by
[turso-apidiff](../../apidiff/) and not filed again.

`CREATE VIRTUAL TABLE IF NOT EXISTS v USING <module>` on a database without `v`
prepares, then fails at the step with `near "IF": syntax error`, and
sqlite_schema stays empty. Without `IF NOT EXISTS` the same statement works.
SQLite 3.50.4 creates the table.

Reproduction: [repro.sql](repro.sql), run with tursodb from the Turso
checkout (it loads Turso's test extension `kv_store`,
`cargo build --release -p turso_cli --bin tursodb -p turso_ext_tests`):
`tursodb -q -m list :memory: < repro.sql`. Output on Turso main 2709799da
([out-turso.txt](out-turso.txt)): the CREATE fails, the first SELECT returns
nothing, and the CREATE without `IF NOT EXISTS` works:

```
Runtime error: near "IF": syntax error
w
```

SQLite 3.50.4 with the equivalent module `memvt`
([../memvt/sqlite/memvt_sqlite.c](../memvt/sqlite/memvt_sqlite.c)) creates
both tables: [repro-sqlite.sql](repro-sqlite.sql),
[out-sqlite.txt](out-sqlite.txt).
