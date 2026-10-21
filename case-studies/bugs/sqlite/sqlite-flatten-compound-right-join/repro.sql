CREATE TABLE t0(c0);
CREATE TABLE t1(c0);
INSERT INTO t0 VALUES (NULL);
-- Expected 7; returns 5 and 7: the first SELECT's WHERE (SELECT 0) is lost.
SELECT * FROM (SELECT 5 FROM t0 WHERE (SELECT 0) UNION ALL SELECT 7 FROM t1 RIGHT JOIN t0 ON 0);
-- Expected no row; returns one NULL: the outer WHERE abs(c0) is lost on the second SELECT.
SELECT * FROM (SELECT c0 FROM t0 UNION ALL SELECT a.c0 FROM t0 AS a RIGHT JOIN t0 AS b ON 0) WHERE abs(c0);
