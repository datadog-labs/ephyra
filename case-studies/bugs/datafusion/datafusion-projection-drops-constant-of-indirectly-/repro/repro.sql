SET datafusion.optimizer.prefer_hash_join = false;
CREATE TABLE t (a INT, b INT) AS VALUES (1, 1);
SELECT t1.b + 1 AS x, t2.a FROM t t1 JOIN t t2 ON t1.a = t2.a WHERE t1.b + 1 = 2 ORDER BY x, t2.a;
