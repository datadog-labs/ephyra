DRAFT, not filed. Comment for existing issue #9073 (not a new issue)

Title (of #9073): A CTE is not found in a subquery inside a JOIN ON clause

Some more data points and the likely cause, on turso main 7641fad2 (2026-10-13):

```sql
CREATE TABLE t(a); CREATE TABLE u(b);
INSERT INTO t VALUES (1); INSERT INTO u VALUES (2);
WITH c AS (SELECT 5 AS x) SELECT * FROM t JOIN u ON (SELECT x FROM c);
-- SQLite 3.50.4: 1|2      turso: Parse error: no such table: c
WITH c AS (SELECT 5 AS x) SELECT * FROM c AS a0 JOIN t ON EXISTS (SELECT * FROM c);
-- SQLite 3.50.4: 5|1      turso: Parse error: no such table: c
WITH c AS (SELECT 5 AS x) SELECT * FROM c JOIN t ON EXISTS (SELECT * FROM c);
-- both: 5|1   (works only because c is joined unaliased)
WITH c AS (SELECT 5 AS x) SELECT (SELECT 1), * FROM t JOIN u ON (SELECT x FROM c);
-- both: 1|1|2 (any other non-FROM subquery makes it work)
```

Recursive and non-recursive CTEs behave the same. LEFT JOIN, the later ON of a 3-way join, and deeper nesting (`ON EXISTS (SELECT * FROM (SELECT * FROM c))`) fail too.

Likely cause: in `core/translate/select.rs`, `select_has_non_from_subqueries` (which sets `preplan_ctes_for_non_from_subqueries` for `parse_from` in `core/translate/planner.rs`) checks the result columns, WHERE, GROUP BY/HAVING, windows, ORDER BY and LIMIT, but not the JOIN ON constraints. So the CTEs are never registered as outer refs for ON-clause subqueries. Those subqueries then find a CTE only through a joined table whose identifier equals the CTE name.
