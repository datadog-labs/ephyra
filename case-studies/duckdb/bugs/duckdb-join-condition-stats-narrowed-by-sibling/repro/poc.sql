SET debug_verify_stats = true;
CREATE TABLE t1(a INT);
INSERT INTO t1 VALUES (0), (1);
CREATE TABLE t2(b INT);
INSERT INTO t2 VALUES (2), (NULL);
SELECT * FROM t1 JOIN t2 ON a < b AND a = b - 1;
