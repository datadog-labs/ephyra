CREATE TABLE t1(a, b, PRIMARY KEY(a, b)) WITHOUT ROWID;
INSERT INTO t1 VALUES(1, 1), (2, 1), (3, 1), (4, 1);
ANALYZE;
SELECT a, b FROM t1 WHERE a = 2 OR (a = 1 AND b IN (1, 2));
-- trunk: 2|1        expected: 2|1  1|1
SELECT a, b FROM t1 WHERE +a = 2 OR (a = 1 AND b IN (1, 2));
-- 1|1  2|1 (ground truth, full scan)

CREATE TABLE t2(x);
INSERT INTO t2 VALUES(1), (2);
SELECT x, a, b FROM t2 CROSS JOIN t1 WHERE (a = 1 AND b IN (1, 2)) OR a = x;
-- trunk: 1|1|1  2|2|1        expected: 1|1|1  2|1|1  2|2|1
