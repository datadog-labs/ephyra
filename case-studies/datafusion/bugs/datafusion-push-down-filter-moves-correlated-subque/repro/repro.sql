CREATE TABLE t1 (c0 BIGINT) AS VALUES (1), (3);
CREATE TABLE t2 (c0 BIGINT, c1 VARCHAR) AS VALUES (1, 'x'), (2, 'y');

-- fails: Optimizer rule ... failed ... No field named a.c0
SELECT * FROM t2 AS a JOIN t1 AS b ON a.c0 = ANY (SELECT c0 FROM t1);

-- same predicate without the table alias: works
SELECT * FROM t2 JOIN t1 AS b ON t2.c0 = ANY (SELECT c0 FROM t1);

-- same predicate in WHERE: works
SELECT * FROM t2 AS a, t1 AS b WHERE a.c0 = ANY (SELECT c0 FROM t1);
