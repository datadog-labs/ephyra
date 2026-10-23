-- Pre-triage check: sqlite-wr-multi-or-seekscan
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Correct build (fix.diff applied; also 3.33.0 and older) prints:
--   q1|2
--   q2|3
-- Buggy builds (3.35.0 .. 3.54.0, trunk 3769f60be38b) print:
--   q1|1
--   q2|2
CREATE TABLE t1(a, b, PRIMARY KEY(a, b)) WITHOUT ROWID;
INSERT INTO t1 VALUES(1, 1), (2, 1), (3, 1), (4, 1);
ANALYZE;
CREATE TABLE t2(x);
INSERT INTO t2 VALUES(1), (2);
SELECT 'q1', count(*) FROM t1 WHERE a = 2 OR (a = 1 AND b IN (1, 2));
SELECT 'q2', count(*) FROM t2 CROSS JOIN t1 WHERE (a = 1 AND b IN (1, 2)) OR a = x;
