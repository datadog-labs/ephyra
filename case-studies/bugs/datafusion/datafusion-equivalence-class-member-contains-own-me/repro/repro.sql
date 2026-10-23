CREATE TABLE t (a INT, b INT) AS VALUES (1, 0), (2, 3);
SET datafusion.optimizer.prefer_hash_join = false;
SELECT * FROM t x JOIN t y ON x.a = y.a AND x.b = y.b WHERE x.a + x.b = x.a;
