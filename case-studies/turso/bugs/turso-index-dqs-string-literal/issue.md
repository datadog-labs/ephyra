Filed as #10337 (https://github.com/tursodatabase/turso/issues/10337) on 2026-10-21.
Title: CREATE INDEX rejects double-quoted string literals that SQLite accepts, so some SQLite databases cannot be opened or written

```sql
CREATE TABLE t(c);
CREATE INDEX i1 ON t(datetime(c, "+1 month"));
CREATE INDEX i2 ON t(c) WHERE c < "-61";
SELECT "-61";
```

Turso:
```
Parse error: Error: invalid expression in CREATE INDEX: datetime (c, "+1 month")
Parse error: Error: cannot use aggregate, window functions or reference other tables in WHERE clause of CREATE INDEX:
 c < "-61"
-61
```
SQLite 3.50.4:
```
-61
```
(both indexes are created)

A database created by SQLite with such an index is affected too:

```
sqlite3 a.db 'CREATE TABLE t(c); CREATE INDEX i1 ON t(datetime(c, "+1 month"));'
tursodb a.db 'SELECT 1;'
-- Turso: Parse error: Error: invalid expression in CREATE INDEX: datetime (c, "+1 month")   (the file does not open)

sqlite3 b.db 'CREATE TABLE t(c); CREATE INDEX i2 ON t(c) WHERE c < "-61";'
tursodb b.db 'INSERT INTO t VALUES (1);'
-- Turso: Parse error: no such column: -61
-- SQLite: ok
```

Expected: a double-quoted identifier that matches no column is taken as a string literal in index expressions and partial-index WHERE clauses, as SQLite does by default (SQLITE_DBCONFIG_DQS_DDL) and as Turso already does in `SELECT "-61"`. SQLite also always accepts this form when it reads an existing schema, even with DQS turned off. With single quotes (`datetime(c, '+1 month')`, `concat_ws('-', c)`, `strftime('%Y', c)`) the same indexes work in Turso.

Likely cause: `validate_index_expression` in `core/translate/index.rs` requires every `Expr::Id` to be a column of the table; the partial-index WHERE check rejects it the same way.

Tested on Turso main cbd5ac4e.
