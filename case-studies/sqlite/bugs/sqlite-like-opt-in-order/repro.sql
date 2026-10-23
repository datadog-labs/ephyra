-- LIKE/GLOB optimization's TEXT-then-BLOB double scan breaks the index
-- order that the planner relies on when a prefix column is iterated by an
-- IN operator (or a skip-scan). ORDER BY / GROUP BY / DISTINCT / compound
-- SELECT merges then give wrong results.
CREATE TABLE t(a, c);
CREATE INDEX ti ON t(a, c);
INSERT INTO t VALUES(1, 'A'), (2, 'A'), (1, x'41');

SELECT a, count(*) FROM t WHERE a IN (1,2) AND c GLOB 'A' GROUP BY a;
-- expected: 1|2  2|1        got: 1|1  2|1  1|1
SELECT DISTINCT a FROM t WHERE a IN (1,2) AND c GLOB 'A';
-- expected: 1  2            got: 1  2  1
SELECT a, quote(c) FROM t WHERE a IN (1,2) AND c GLOB 'A' ORDER BY a;
-- expected: 1|'A' 1|X'41' 2|'A' (in some order within a=1)
-- got:      1|'A' 2|'A' 1|X'41'
SELECT a, quote(c) FROM (SELECT a, c FROM t WHERE a IN (1,2) AND c GLOB 'A'
                         INTERSECT SELECT a, c FROM t);
-- expected: 3 rows          got: 1|'A' 2|'A'  (blob row lost)
SELECT a, count(*) FROM t WHERE +a IN (1,2) AND c GLOB 'A' GROUP BY a;
-- reference without the index on a: 1|2  2|1
