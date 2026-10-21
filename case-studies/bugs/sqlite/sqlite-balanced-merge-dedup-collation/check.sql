-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-balanced-merge-dedup-collation
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t0(c0);
CREATE TABLE t1(c0 COLLATE RTRIM);
INSERT INTO t1 VALUES (''), (' ');
CREATE TABLE t(c);
INSERT INTO t VALUES ('a'), ('A');
SELECT 'q1';
.mode quote
SELECT c0 FROM t0 UNION SELECT 1 UNION SELECT 2 UNION SELECT c0 FROM t1;
.mode list
SELECT 'q2';
.mode quote
SELECT c0 FROM t0 UNION SELECT 1 UNION SELECT 2 UNION SELECT c0 FROM t1 ORDER BY 1;
.mode list
SELECT 'q3';
.mode quote
SELECT 1 COLLATE BINARY UNION SELECT 2 UNION SELECT '' UNION SELECT ' ' COLLATE RTRIM EXCEPT SELECT 9;
.mode list
SELECT 'q4';
.mode quote
SELECT c || '' FROM t UNION SELECT 'b' UNION ALL SELECT 'x' COLLATE NOCASE ORDER BY 1;
.mode list
