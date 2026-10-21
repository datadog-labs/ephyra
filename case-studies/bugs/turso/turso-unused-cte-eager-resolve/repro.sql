CREATE TABLE t(a);
INSERT INTO t VALUES (1);
-- accepted: no subquery outside FROM
WITH x AS (SELECT nosuch FROM t) SELECT 1;
-- rejected: unused CTE x is resolved because the SELECT has a subquery
WITH x AS (SELECT nosuch FROM t) SELECT (SELECT 1);
WITH x AS (SELECT * FROM nosuchtab) SELECT a FROM t WHERE a IN (SELECT 1);
WITH x AS (SELECT nosuch FROM t) UPDATE t SET a = (SELECT 2);
SELECT a FROM t;
