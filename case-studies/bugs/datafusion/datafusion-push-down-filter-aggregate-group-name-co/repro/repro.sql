SET datafusion.optimizer.skip_failed_rules = true;
CREATE TABLE t (a INT) AS VALUES (1), (2);
SELECT a FROM t GROUP BY a, CAST(a AS VARCHAR) HAVING a > (SELECT min(a) FROM t);
