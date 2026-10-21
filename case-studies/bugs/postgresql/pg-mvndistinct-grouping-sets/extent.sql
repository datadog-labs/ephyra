\set VERBOSITY terse
\set ON_ERROR_STOP 0
DROP TABLE IF EXISTS t0;
CREATE TABLE t0 (c0 int8, c1 int8);
INSERT INTO t0 VALUES (NULL, NULL);
CREATE STATISTICS t0_s ON c0, c1 FROM t0;
ANALYZE t0;
-- A: grouping forms
\echo A1 rollup
SELECT c0 FROM t0 GROUP BY c0, ROLLUP (abs(c0));
\echo A2 cube
SELECT c0 FROM t0 GROUP BY c0, CUBE (abs(c0));
\echo A3 grouping sets
SELECT c0 FROM t0 GROUP BY GROUPING SETS ((c0, abs(c0)), (c0));
\echo A4 grouping sets single set
SELECT c0 FROM t0 GROUP BY GROUPING SETS ((c0, abs(c0)));
\echo A5 plain group by
SELECT c0 FROM t0 GROUP BY c0, abs(c0);
\echo A6 rollup of both
SELECT c0 FROM t0 GROUP BY ROLLUP (c0, abs(c0));
\echo A7 rollup c0 only
SELECT c0 FROM t0 GROUP BY abs(c0), ROLLUP (c0);
\echo A8 distinct
SELECT DISTINCT c0, abs(c0) FROM t0;
\echo A9 grouping sets with c1
SELECT c0 FROM t0 GROUP BY c0, ROLLUP (c1);
-- B: expressions
\echo B1 c0+1
SELECT c0 FROM t0 GROUP BY c0, ROLLUP (c0 + 1);
\echo B2 cast
SELECT c0 FROM t0 GROUP BY c0, ROLLUP (c0::text);
\echo B3 cast numeric
SELECT c0 FROM t0 GROUP BY c0, ROLLUP (c0::numeric);
\echo B4 two-col expr c0+c1
SELECT c0 FROM t0 GROUP BY c0, ROLLUP (c0 + c1);
\echo B5 two-col expr c1+c0, group by c1
SELECT c1 FROM t0 GROUP BY c1, ROLLUP (c0 + c1);
\echo B6 expr of c1 grouped with c0
SELECT c0 FROM t0 GROUP BY c0, ROLLUP (abs(c1));
\echo B7 c0, c1, rollup abs(c0)
SELECT c0 FROM t0 GROUP BY c0, c1, ROLLUP (abs(c0));
\echo B8 rollup abs(c0) abs(c0)+1
SELECT 1 FROM t0 GROUP BY ROLLUP (abs(c0), c0 + 1);
