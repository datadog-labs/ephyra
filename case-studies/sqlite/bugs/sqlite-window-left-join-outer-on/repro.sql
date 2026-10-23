CREATE TABLE t(c);
INSERT INTO t VALUES (NULL);
-- Expected 1|1 (the LEFT JOIN keeps b's row whatever its ON clause); returns 1|0.
SELECT count(*) OVER (), (SELECT count(*) FROM t AS b LEFT JOIN t AS d ON a.c) FROM t AS a;
-- Without the window function: 1, right.
SELECT (SELECT count(*) FROM t AS b LEFT JOIN t AS d ON a.c) FROM t AS a;
