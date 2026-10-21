-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-rowvalue-in-join-reuse
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t1(a PRIMARY KEY, b);
INSERT INTO t1 VALUES(1, 1);
CREATE TABLE t2(p, q);
INSERT INTO t2 VALUES(1, 1), (1, 2), (1, 3);
CREATE TABLE t3(c0, PRIMARY KEY(c0)) WITHOUT ROWID;
INSERT INTO t3 VALUES('k');
ANALYZE;
SELECT 'q1';
SELECT count(*) FROM t1 x, t1 y WHERE x.a = y.a AND (x.a, x.b) IN ((1, 1), (1, 2));
SELECT 'q2';
SELECT x.a, x.b FROM t1 x, t1 y WHERE x.a = y.a AND (x.a, x.b) IN ((1, 1), (1, 2), (1, 3));
SELECT 'q3';
SELECT count(*) FROM t1 x, t1 y WHERE x.a = y.a AND (x.a, x.b) IN (VALUES(1, 1), (1, 2));
SELECT 'q4';
SELECT count(*) FROM t1 x, t1 y WHERE x.a = y.a AND (x.a, x.b) IN (SELECT p, q FROM t2);
SELECT 'q5';
SELECT count(*) FROM t3 AS a0 JOIN t3 AS a1 USING (c0) WHERE (a0.c0, a0.c0) IN (('k', NULL), ('k', 'k'));
