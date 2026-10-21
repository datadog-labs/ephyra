DRAFT, not filed

Title: An unused CTE is resolved, and its errors reported, when the statement has a subquery outside FROM

SQLite does not resolve a CTE that the statement never reads, so an invalid name in an unused CTE body is not an error. Turso does the same for simple statements. But once the statement has any subquery outside the FROM clause, Turso plans every CTE up front and reports the unused CTE's errors.

```sql
CREATE TABLE t(a);
INSERT INTO t VALUES (1);
WITH x AS (SELECT nosuch FROM t) SELECT 1;
WITH x AS (SELECT nosuch FROM t) SELECT (SELECT 1);
WITH x AS (SELECT * FROM nosuchtab) SELECT a FROM t WHERE a IN (SELECT 1);
WITH x AS (SELECT nosuch FROM t) UPDATE t SET a = (SELECT 2);
SELECT a FROM t;
```

`tursodb` (main 7641fad2):
```
1
  × Parse error: no such column: nosuch
  × Parse error: no such table: nosuchtab
  × Parse error: no such column: nosuch
1
```

SQLite 3.50.4 (and trunk):
```
1
1
1
2
```

Expected: SQLite's output. The unused CTE `x` is never resolved.

It is the same for subqueries in ORDER BY, LIMIT, HAVING, INSERT ... SELECT, DELETE ... WHERE, CREATE VIEW, and for a WITH inside a scalar subquery.

Likely cause: `parse_from` in `core/translate/planner.rs` calls `plan_cte` for every CTE definition when `preplan_ctes_for_non_from_subqueries` is set (from `select_has_non_from_subqueries` in `core/translate/select.rs`), so it can register them as outer refs for non-FROM subqueries. Errors in CTEs that are never referenced come out of that eager planning. SQLite only resolves a CTE when a FROM item references it.

Found by differential testing against SQLite 3.50.4. Turso commit tested: 7641fad2.
