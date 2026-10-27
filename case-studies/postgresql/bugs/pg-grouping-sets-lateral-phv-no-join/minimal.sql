CREATE TABLE t (x int);
INSERT INTO t VALUES (1), (2);
-- valid query; fails with "failed to build any 4-way joins" (XX000)
SELECT s.y
FROM t b LEFT JOIN t a ON a.x = b.x
     CROSS JOIN LATERAL (SELECT a.x AS y FROM t c, t d) s
GROUP BY ROLLUP (s.y);
-- control: the same query without grouping sets plans and runs
SELECT s.y
FROM t b LEFT JOIN t a ON a.x = b.x
     CROSS JOIN LATERAL (SELECT a.x AS y FROM t c, t d) s;
