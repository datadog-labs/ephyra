CREATE TABLE t(a);
WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i < 600) INSERT INTO t SELECT i FROM c;
-- slow: ON uses IS, so no probe index and the subquery runs once per outer row
SELECT count(*) FROM t AS x LEFT JOIN (SELECT max(b.a) AS m FROM t AS b, t AS c) AS s ON s.m IS x.a;
-- fast: = gets an ephemeral index; AS MATERIALIZED is fast too
SELECT count(*) FROM t AS x LEFT JOIN (SELECT max(b.a) AS m FROM t AS b, t AS c) AS s ON s.m = x.a;
