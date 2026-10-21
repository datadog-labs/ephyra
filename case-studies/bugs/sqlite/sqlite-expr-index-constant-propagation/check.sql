-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-expr-index-constant-propagation
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t1(c TEXT);
INSERT INTO t1 VALUES('');
CREATE INDEX t1c ON t1(CAST(c AS INTEGER));
SELECT 'q1';
SELECT count(*) FROM (SELECT 1) CROSS JOIN t1
 WHERE CAST(c AS INTEGER) IN (SELECT 0) AND c = '';
CREATE TABLE t2(a INTEGER PRIMARY KEY);
INSERT INTO t2 VALUES(-1),(0);
CREATE INDEX t2a ON t2(CAST(a AS INTEGER));
SELECT 'q2';
SELECT * FROM t2 AS x CROSS JOIN t2 AS y INDEXED BY t2a
 WHERE x.a <> CAST(y.a AS INTEGER) AND y.a = 0;
CREATE TABLE t3(a INT);
INSERT INTO t3 VALUES(0);
CREATE INDEX t3a ON t3(a+1);
SELECT 'q3';
SELECT count(*) FROM (SELECT 1) CROSS JOIN t3
 WHERE a+1 IN (SELECT 1) AND a = 0;
