CREATE TABLE t1(c0 INTEGER); INSERT INTO t1 VALUES (1);
CREATE TABLE t2(c3 BOOLEAN); INSERT INTO t2 VALUES (NULL), (true);
SELECT 'default' AS mode, count(*) AS n FROM (SELECT 2 AS a0 FROM t1 AS a JOIN t1 AS b ON a.c0 = b.c0 SEMI JOIN t2 ON t2.c3);
SET disabled_optimizers='filter_pushdown';
SELECT 'no filter_pushdown' AS mode, count(*) AS n FROM (SELECT 2 AS a0 FROM t1 AS a JOIN t1 AS b ON a.c0 = b.c0 SEMI JOIN t2 ON t2.c3);
