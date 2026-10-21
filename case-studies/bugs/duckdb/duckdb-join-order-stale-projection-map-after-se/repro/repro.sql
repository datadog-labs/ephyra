CREATE TABLE t(c DOUBLE);
INSERT INTO t VALUES (0.0), (NULL);
SET disabled_optimizers='empty_result_pullup';
SELECT * FROM (SELECT 0.5 AS a FROM (SELECT 1 AS a WHERE false) AS x4 CROSS JOIN t AS x5 SEMI JOIN t AS x6 ON true WHERE coalesce((SELECT x4.a), x5.c) IS NOT NULL) AS x9 JOIN t AS x1 ON x1.c < x9.a;
