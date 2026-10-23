CREATE TABLE t0(a);
CREATE TABLE t1(a, b);
CREATE INDEX t1a ON t1(a);
CREATE TABLE t2(c);
INSERT INTO t0 VALUES(0);
INSERT INTO t1 VALUES(0, 1);
ANALYZE;
INSERT INTO t2 VALUES(9);
SELECT * FROM t0 JOIN t1 ON t1.a IN (t0.a) RIGHT JOIN t2 ON 1
 WHERE t1.b IS 5 OR t1.a IS 5 ORDER BY t1.b;
-- trunk: 0|0|1|9        expected: no rows (t1.b is 1, t1.a is 0)
SELECT * FROM t0 JOIN t1 ON t1.a IN (t0.a) RIGHT JOIN t2 ON 1
 WHERE +(t1.b IS 5 OR t1.a IS 5) ORDER BY t1.b;
-- no rows (ground truth: the same WHERE clause, not an OR term)
