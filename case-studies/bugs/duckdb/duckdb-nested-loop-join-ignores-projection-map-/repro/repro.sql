CREATE TABLE t0(c0 INTEGER);
INSERT INTO t0 VALUES (1), (2);
-- expected: 4 rows: 1,2 / 1,2 / 2,1 / 2,1 (the LEFT JOIN condition is true, so t0 x t1 has 4 rows;
-- the same query with ON (true) returns exactly these rows)
SELECT t0.c0, t2.c0 FROM t0 LEFT JOIN t0 t1 ON (1 IN (SELECT 1)) JOIN t0 t2 ON (t0.c0 <> t2.c0) ORDER BY ALL;
