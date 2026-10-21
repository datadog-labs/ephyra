-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-wr-bignull-added-column
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
-- The error case prints "Error near line N: database disk image is malformed" on stderr;
-- run with 2>&1 to capture it.
CREATE TABLE t1(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t1 VALUES(1.5), (2);
ALTER TABLE t1 ADD COLUMN b;
INSERT INTO t1 VALUES(3, NULL), (4, 7);
CREATE TABLE t2(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t2 VALUES(1), (2);
ALTER TABLE t2 ADD COLUMN b DEFAULT 5;
INSERT INTO t2 VALUES(3, NULL);
CREATE TABLE t3(a, b, PRIMARY KEY(a, b)) WITHOUT ROWID;
INSERT INTO t3 VALUES(1, 1), (1, 2);
ALTER TABLE t3 ADD COLUMN c;
SELECT 'q1';
SELECT a, b FROM t1 WHERE a IN (1.5, 3) ORDER BY b NULLS LAST;
SELECT 'q2';
SELECT a, b FROM t1 WHERE a IN (1.5, 2, 4) ORDER BY b DESC NULLS FIRST;
SELECT 'q3';
SELECT a, b FROM t2 WHERE a IN (1, 2, 3) ORDER BY b NULLS LAST;
SELECT 'q4';
SELECT a, b, c FROM t3 WHERE a = 1 AND b IN (1, 2) ORDER BY c NULLS LAST;
