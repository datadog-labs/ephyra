Filed as #10396 (https://github.com/tursodatabase/turso/issues/10396) on 2026-10-21.
Title: `schema.alias.column` that refers to an outer query's table is not found in a subquery ("no such column: main.x.a")

```sql
CREATE TABLE t(a);
INSERT INTO t VALUES (5);
SELECT main.x.a FROM t AS x;
SELECT (SELECT main.x.a) FROM t AS x;
SELECT * FROM t AS x WHERE EXISTS (SELECT 1 WHERE main.x.a = 5);
SELECT (SELECT count(*) FROM json_each(main.x.a)) FROM t AS x;
SELECT (SELECT main.t.a) FROM t;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
5
  × Parse error: no such column: main.x.a
  × Parse error: no such column: main.x.a
  × Parse error: no such column: main.x.a
5
```

SQLite 3.50.4 (trunk gives the same):
```
5
5
5
1
5
```

Expected: SQLite accepts a schema name in front of a table alias, and Turso does too in the query that owns the alias (first statement). From inside a subquery the same name is not found. Without an alias (`main.t.a`, last statement) the subquery works.

Likely cause: in `resolve_qualified_name` (core/translate/expr/binding.rs), the loop over the query's own tables passes `None` instead of the database id when the id matches that table's database, so the alias can match. The loop over `outer_query_refs()` passes `database_id` unchanged to `match_qualified_name_in_table`, so an alias in an outer query never matches a schema-qualified name.

Tested on Turso main db441331.
