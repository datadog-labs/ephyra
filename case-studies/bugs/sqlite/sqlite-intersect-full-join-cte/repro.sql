-- Wrong result: inner-join ON term to the left of a RIGHT/FULL JOIN is lost in all but
-- one arm when a UNION ALL subquery elsewhere in the FROM clause is flattened.
-- Trunk 0e707e7dba7e (2026-10-10 00:11), 3.53.4, and every release since 3.51.0.
CREATE TABLE t1(a);
INSERT INTO t1 VALUES(1);

SELECT z.a, v FROM t1 AS x JOIN t1 AS y ON 0 RIGHT JOIN t1 AS z
  CROSS JOIN (SELECT 2 AS v FROM t1 UNION ALL SELECT 3 FROM t1);
-- Expected (DuckDB agrees; also SQLite with .testctrl optimizations -FlttnUnionAll):
--   1|2
--   1|3
-- Actual:
--   1|3

-- Original fuzz finding (TLP intersect oracle), for reference:
-- CREATE TABLE t2(c0); with t1(c0) holding 0 and t2 empty
-- WITH x0 AS (SELECT c0 FROM t1 UNION ALL
--             SELECT t2.c0 FROM t2 JOIN t1 AS a1 ON NULL FULL JOIN t1 AS a2 ON NULL)
-- SELECT * FROM x0 AS a0 CROSS JOIN x0 AS a1;
-- Expected 0|0, 0|NULL, NULL|0, NULL|NULL; actual 0|0, NULL|0.
-- (DISTINCT hides it because restriction 17d2 then blocks the flattening.)
