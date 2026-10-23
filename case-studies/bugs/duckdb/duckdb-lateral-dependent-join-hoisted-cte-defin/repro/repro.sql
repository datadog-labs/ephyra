CREATE TABLE t(x INT);
INSERT INTO t VALUES (1), (2), (NULL);
SELECT count(*) FROM t a, (SELECT (SELECT x) y FROM t) b FULL JOIN t c ON (SELECT a.x + b.y + c.x) = 4;
