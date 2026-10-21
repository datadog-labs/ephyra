-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-intersect-full-join-cte
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t1(a);
INSERT INTO t1 VALUES(1);
SELECT 'q1';
SELECT z.a, v FROM t1 AS x JOIN t1 AS y ON 0 RIGHT JOIN t1 AS z
  CROSS JOIN (SELECT 2 AS v FROM t1 UNION ALL SELECT 3 FROM t1) ORDER BY v;
CREATE TABLE t0(c0);
CREATE TABLE t2(c0);
INSERT INTO t0 VALUES(0);
SELECT 'q2';
WITH x0 AS (SELECT c0 FROM t0 UNION ALL
            SELECT t2.c0 FROM t2 JOIN t0 AS a1 ON NULL FULL JOIN t0 AS a2 ON NULL)
SELECT quote(a0.c0), quote(a1.c0) FROM x0 AS a0 CROSS JOIN x0 AS a1 ORDER BY 1, 2;
