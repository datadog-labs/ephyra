# CREATE VIRTUAL TABLE IF NOT EXISTS fails with 'near "IF": syntax error' and creates nothing

Reported by others as https://github.com/tursodatabase/turso/issues/11352
(2026-11-04); found independently by
[turso-apidiff](../../apidiff/) and not filed again.

`CREATE VIRTUAL TABLE IF NOT EXISTS v USING mem` on a database without `v`
prepares, then fails at the step with `near "IF": syntax error`, and
sqlite_schema stays empty. Without `IF NOT EXISTS` the same statement works.
SQLite 3.50.4 creates the table.

Reproduction: [repro.rs](repro.rs), the binary `vtab_if_not_exists` of
[apidiff/repro2](../../apidiff/repro2/) (`cargo run --release --bin
vtab_if_not_exists` there; `mem` is the minimal native module in
repro2/src/lib.rs). Output on Turso main 52d6fd81d:

```
CREATE VIRTUAL TABLE IF NOT EXISTS v USING mem
  -> error: near "IF": syntax error
SELECT name, sql FROM sqlite_schema
CREATE VIRTUAL TABLE w USING mem
SELECT name, sql FROM sqlite_schema
  -> row w|CREATE VIRTUAL TABLE w  USING mem
 /*w(a, b)*/
```
