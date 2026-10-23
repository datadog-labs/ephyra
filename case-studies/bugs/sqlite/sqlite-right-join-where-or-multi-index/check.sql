-- Pre-triage check: sqlite-right-join-where-or-multi-index
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Correct build (fix.diff applied) prints:
--   q1|0
--   q2|0
-- Buggy builds (3.39.0 .. 3.54.0, trunk 03f699544e6b) print:
--   q1|1
--   q2|1
CREATE TABLE t0(a);
CREATE TABLE t1(a, b);
CREATE INDEX t1a ON t1(a);
CREATE TABLE t2(c);
INSERT INTO t0 VALUES(0);
INSERT INTO t1 VALUES(0, 1);
ANALYZE;
INSERT INTO t2 VALUES(9);
SELECT 'q1', count(*) FROM (SELECT t1.b FROM t0 JOIN t1 ON t1.a IN (t0.a)
  RIGHT JOIN t2 ON 1 WHERE t1.b IS 5 OR t1.a IS 5 ORDER BY t1.b);
.output /dev/null
.testctrl optimizations 0x2000
.output
SELECT 'q2', count(*) FROM (SELECT t1.b FROM t0 JOIN t1 ON t1.a IN (t0.a)
  FULL JOIN t2 ON NULL WHERE t1.b = 5 OR t1.a = 5 ORDER BY t1.b);
