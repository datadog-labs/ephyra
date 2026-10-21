-- Sorted grouping sets: a grouping column of another set is not nulled
-- when the input tuple has a NULL and a later column is read.
CREATE TEMP TABLE t (id int PRIMARY KEY, a int, b int, c int);
INSERT INTO t VALUES (1, 10, NULL, 100);
SET enable_hashagg = off;
-- expected: (10, NULL, 100, 1) and (NULL, NULL, 100, 2)
SELECT a, b, c, GROUPING(a, b) AS g FROM t GROUP BY GROUPING SETS ((id, a), (id, b));
EXPLAIN (VERBOSE, COSTS OFF)
SELECT a, b, c, GROUPING(a, b) AS g FROM t GROUP BY GROUPING SETS ((id, a), (id, b));
-- control: hashed
RESET enable_hashagg;
SELECT a, b, c, GROUPING(a, b) AS g FROM t GROUP BY GROUPING SETS ((id, a), (id, b));
SET enable_hashagg = off;
-- control: no NULL in the row
UPDATE t SET b = 20;
SELECT a, b, c, GROUPING(a, b) AS g FROM t GROUP BY GROUPING SETS ((id, a), (id, b));
-- control: c not selected
UPDATE t SET b = NULL;
SELECT a, b, GROUPING(a, b) AS g FROM t GROUP BY GROUPING SETS ((id, a), (id, b));
