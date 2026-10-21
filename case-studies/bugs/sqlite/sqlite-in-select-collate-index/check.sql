-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-in-select-collate-index
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t1(a);
INSERT INTO t1 VALUES('a'), ('A'), ('b '), ('b');
CREATE TABLE t2(b);
INSERT INTO t2 VALUES('A'), ('b');
CREATE INDEX t1a ON t1(a);
SELECT 'q1';
SELECT quote(a) FROM t1 WHERE a IN (SELECT b COLLATE NOCASE FROM t2) ORDER BY a;
SELECT 'q2';
SELECT quote(a) FROM t1 WHERE a IN (SELECT b COLLATE RTRIM FROM t2) ORDER BY a;
SELECT 'q3';
SELECT quote(a) FROM t1 WHERE a IN (SELECT max(b COLLATE NOCASE) FROM t2 WHERE b = 'A') ORDER BY a;
