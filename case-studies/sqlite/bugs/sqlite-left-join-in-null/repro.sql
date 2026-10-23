CREATE TABLE t(a, b, c);
INSERT INTO t VALUES (1, NULL, 5), (2, NULL, NULL);
CREATE INDEX i ON t(b, c);
SELECT x.a, y.a FROM t AS x LEFT JOIN t AS y ON y.b = x.c AND y.c IN (5, 0);
