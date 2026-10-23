CREATE TABLE t(a, b);
INSERT INTO t VALUES (1, 2);
SELECT * FROM t WHERE (a, b) BETWEEN (0, 0) AND (5, 5);
SELECT * FROM t AS x JOIN t AS y ON x.a = y.a AND (x.a, x.b) BETWEEN (0, 0) AND (5, 5);
SELECT * FROM t AS x LEFT JOIN t AS y ON (y.a, y.b) BETWEEN (0, 0) AND (5, 5);
