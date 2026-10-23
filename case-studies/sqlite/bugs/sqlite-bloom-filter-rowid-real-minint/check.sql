-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-bloom-filter-rowid-real-minint
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t1(a);
INSERT INTO t1 VALUES(-9223372036854775808.0);
CREATE TABLE t2(b INTEGER PRIMARY KEY);
INSERT INTO t2 VALUES(-9223372036854775808);
ANALYZE;
UPDATE sqlite_stat1 SET stat='100' WHERE tbl='t1';
UPDATE sqlite_stat1 SET stat='10' WHERE tbl='t2';
ANALYZE sqlite_schema;
SELECT 'q1';
SELECT count(*) FROM t1, t2 WHERE b = a AND b <> 5;
PRAGMA foreign_keys = ON;
CREATE TABLE p(id INTEGER PRIMARY KEY);
INSERT INTO p VALUES(-9223372036854775808);
CREATE TABLE c(r REFERENCES p);
INSERT INTO c VALUES(-9223372036854775808.0);
SELECT 'q2';
SELECT count(*) FROM c JOIN p ON p.id = c.r;
