-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-leftjoin-on-or-omitted-table-segv
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
-- On an affected build q1 prints 5, 6 and q2 crashes (SIGSEGV, nothing after "q2").
CREATE TABLE t1(a, b);
CREATE INDEX t1b ON t1(b);
CREATE TABLE t2(c);
INSERT INTO t1 VALUES(1,1),(2,1),(3,1),(4,1),(5,1),(6,1);
ANALYZE;
SELECT 'q1';
SELECT DISTINCT y.a FROM t1 AS x JOIN t1 AS y ON y.b >= x.b AND y.b <= x.b
  LEFT JOIN t2 ON y.a = 5 OR y.a = 6
 WHERE y.b BETWEEN 1 AND 2 AND y.b = x.b;
SELECT 'q2';
SELECT DISTINCT y.a FROM t1 AS x JOIN t1 AS y ON y.b >= x.b AND y.b <= x.b
  LEFT JOIN t2 ON t2.c = y.a OR y.a = 5
 WHERE y.b BETWEEN 1 AND 2 AND y.b = x.b;
