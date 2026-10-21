-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-wr-rowvalue-added-column
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t1(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t1 VALUES(1.5), (2.5);
ALTER TABLE t1 ADD COLUMN b DEFAULT 5;
CREATE TABLE t2(x);
INSERT INTO t2 VALUES(0);
CREATE TABLE t3(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t3 VALUES(1), (2), (3);
ALTER TABLE t3 ADD COLUMN b DEFAULT 5;
CREATE TABLE t4(x, y, PRIMARY KEY(x, y)) WITHOUT ROWID;
INSERT INTO t4 VALUES(1, 1), (1, 2);
ALTER TABLE t4 ADD COLUMN z;
INSERT INTO t4 VALUES(1, 3, 7);
SELECT 'q1';
SELECT * FROM t1 WHERE (a, b) > (1.5, 0);
SELECT 'q2';
SELECT * FROM t1 WHERE (a, b) < (2.5, 9);
SELECT 'q3';
SELECT * FROM t2 RIGHT JOIN t1 ON (t1.a, t1.b) > (1.5, 0);
SELECT 'q4';
SELECT * FROM t3 WHERE (a, b) > (1, 0) AND (a, b) < (3, 9);
SELECT 'q5';
SELECT * FROM t4 WHERE (x, y, z) > (1, 1, NULL);
