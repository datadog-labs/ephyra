-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-wr-in-index-added-column
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t1(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t1 VALUES(1.5), (2.5);
ALTER TABLE t1 ADD COLUMN b DEFAULT 5;
CREATE TABLE t2(x, y);
INSERT INTO t2 VALUES(1.5, 5), (2.5, 6);
CREATE INDEX t2xy ON t2(x, y);
CREATE TABLE t3(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t3 VALUES(1), (2), (3);
ALTER TABLE t3 ADD COLUMN b DEFAULT 5;
CREATE TABLE t4(x, y, PRIMARY KEY(x, y)) WITHOUT ROWID;
INSERT INTO t4 VALUES(1, 1), (1.5, 2);
ALTER TABLE t4 ADD COLUMN z DEFAULT 7;
SELECT 'q1';
SELECT (1.5, 5) IN (SELECT a, b FROM t1);
SELECT 'q2';
SELECT (1.5, 6) NOT IN (SELECT a, b FROM t1);
SELECT 'q3';
SELECT (2, 5) IN (SELECT a, b FROM t3);
SELECT 'q4';
SELECT * FROM t2 WHERE (x, y) IN (SELECT a, b FROM t1);
SELECT 'q5';
SELECT (1.5, 2, 7) IN (SELECT x, y, z FROM t4);
