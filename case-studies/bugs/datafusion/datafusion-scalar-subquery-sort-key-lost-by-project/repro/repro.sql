SET datafusion.execution.target_partitions = 4;
SET datafusion.execution.batch_size = 1;
CREATE TABLE t0 (c1 BOOLEAN, c2 BIGINT);
INSERT INTO t0 VALUES (true, 3), (true, 2), (false, NULL);
CREATE TABLE t1 (c2 BIGINT);
SELECT a2.c1 AS x0, (SELECT max(c2) FROM t0) AS x1
FROM t1 AS a1 RIGHT JOIN t0 AS a2 ON a1.c2 = a2.c2
ORDER BY 1 DESC, 2 LIMIT 5;
