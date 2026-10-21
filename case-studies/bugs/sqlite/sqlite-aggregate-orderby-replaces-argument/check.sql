-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-aggregate-orderby-replaces-argument
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t1(a);
CREATE TABLE t2(b);
INSERT INTO t1 VALUES(1);
INSERT INTO t2 VALUES(2);
SELECT 'q1';
SELECT sum(b ORDER BY a) FROM t1, t2;
CREATE TABLE t3(c0, c1);
INSERT INTO t3 VALUES(1, 2);
SELECT 'q2';
SELECT count(a1.c1 ORDER BY a0.c1) FROM t3 AS a0 LEFT JOIN t3 AS a1 ON NULL;
CREATE TABLE t0(c0, c1);
INSERT INTO t0 VALUES(NULL, NULL), ('', 0);
SELECT 'q3';
SELECT quote(a0.c0), count(a1.c1 ORDER BY a0.c1) FROM t0 AS a0 CROSS JOIN t0 AS a1 GROUP BY a0.c0;
CREATE TABLE t4(k, v);
INSERT INTO t4 VALUES(1, 'x'), (2, 'y');
SELECT 'q4';
SELECT group_concat(q.v ORDER BY p.v) FROM t4 AS p, t4 AS q WHERE p.k<>q.k;
