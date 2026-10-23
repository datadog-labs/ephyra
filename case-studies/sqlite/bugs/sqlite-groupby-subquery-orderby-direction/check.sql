-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-groupby-subquery-orderby-direction
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t(a);
INSERT INTO t VALUES(1),(2);
SELECT 'q1';
SELECT a FROM (SELECT a FROM t ORDER BY a LIMIT 2) GROUP BY a ORDER BY a DESC;
SELECT 'q2';
SELECT a FROM (SELECT a FROM t ORDER BY a DESC LIMIT 2) GROUP BY a ORDER BY a;
CREATE TABLE t0(c0 INTEGER PRIMARY KEY);
INSERT INTO t0 VALUES(1),(2);
SELECT 'q3';
WITH x0 AS (SELECT c0 FROM t0 ORDER BY rowid LIMIT 2) SELECT c0 FROM x0 GROUP BY c0 ORDER BY 1 DESC;
