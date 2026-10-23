CREATE TABLE t(c DOUBLE);
INSERT INTO t VALUES (-1), (0);
SELECT c, if(c > 5, 5, abs(c)) = 0 AS r FROM t ORDER BY c;
