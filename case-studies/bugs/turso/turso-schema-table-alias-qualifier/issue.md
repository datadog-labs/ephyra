Filed as #10353 (https://github.com/tursodatabase/turso/issues/10353) on 2026-10-21.
Title: A column qualified with sqlite_master is not found when FROM names sqlite_schema (and the reverse)

```sql
CREATE TABLE t(a);
SELECT sqlite_master.name FROM sqlite_schema;
SELECT sqlite_schema.name FROM sqlite_master;
SELECT sqlite_master.name FROM main.sqlite_schema;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
Parse error: no such table: sqlite_master
Parse error: no such table: sqlite_schema
Parse error: no such table: sqlite_master
```

SQLite 3.50.4 (trunk gives the same):
```
t
t
t
```

Expected: the SQLite output. SQLite treats `sqlite_schema` and `sqlite_master` (and, for the temp schema, `sqlite_temp_schema` and `sqlite_temp_master`) as names of the same table when matching a column qualifier, so either spelling qualifies a column whatever name the FROM clause used (https://sqlite.org/schematab.html). Turso accepts both names in FROM, but a qualifier only matches the spelling used there. `SELECT sqlite_temp_master.name FROM sqlite_temp_schema` fails the same way.

Likely cause: table lookup maps the alternative names to `sqlite_schema` (`Schema::normalize_table_lookup_name`, core/schema.rs), but qualified column binding (core/translate/expr/binding.rs) compares the qualifier with the FROM table's name as written, without that mapping.

Related to #1641, which is about the alternative names in FROM; this one is about column qualifiers.

Tested on Turso main cbd5ac4e.
