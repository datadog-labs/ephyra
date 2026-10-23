CREATE TABLE t0(x INT);
CREATE TABLE t1(a INT, b INT);
CREATE TABLE t3(k INTEGER PRIMARY KEY);
INSERT INTO t0 SELECT value%3 FROM generate_series(1,5);
INSERT INTO t1 SELECT value%5, value FROM generate_series(1,50);
INSERT INTO t3 SELECT value*2 FROM generate_series(1,20);
ANALYZE;
SELECT y, (SELECT count(*) FROM t0, t1, t3 WHERE t1.a = t0.x AND t3.k = t1.b + y)
  FROM (SELECT 0 AS y UNION ALL SELECT 1);
