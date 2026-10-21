CREATE TABLE t(a);
CREATE TABLE u(b);
INSERT INTO t VALUES (1);
INSERT INTO u VALUES (2);
-- fails: CTE read only inside an ON-clause subquery
WITH c AS (SELECT 5 AS x) SELECT * FROM t JOIN u ON (SELECT x FROM c);
-- fails: CTE joined under an alias, subquery uses the CTE name
WITH c AS (SELECT 5 AS x) SELECT * FROM c AS a0 JOIN t ON EXISTS (SELECT * FROM c);
-- works: CTE joined unaliased
WITH c AS (SELECT 5 AS x) SELECT * FROM c JOIN t ON EXISTS (SELECT * FROM c);
-- works: any other subquery in the SELECT makes the CTEs visible
WITH c AS (SELECT 5 AS x) SELECT (SELECT 1), * FROM t JOIN u ON (SELECT x FROM c);
