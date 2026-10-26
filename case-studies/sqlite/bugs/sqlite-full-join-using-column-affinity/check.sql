CREATE TABLE t1(a REAL);
CREATE TABLE t2(a);
INSERT INTO t2 VALUES(0), (9007199254740993);
-- the USING column of a FULL JOIN, read directly: integers, as stored in t2
SELECT 'q1';
SELECT a, typeof(a) FROM t1 FULL JOIN t2 USING(a);
-- the same rows through a subquery that is flattened: the same
SELECT 'q2';
SELECT a, typeof(a) FROM (SELECT a FROM t1 FULL JOIN t2 USING(a));
-- the same rows through a materialized CTE / DISTINCT subquery: REAL, precision lost
SELECT 'q3';
WITH c AS MATERIALIZED (SELECT a FROM t1 FULL JOIN t2 USING(a))
  SELECT a, typeof(a) FROM c;
SELECT 'q4';
SELECT a, typeof(a) FROM (SELECT DISTINCT a FROM t1 FULL JOIN t2 USING(a));
