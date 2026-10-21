-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-vtab-create-rollback-to-savepoint
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
CREATE TABLE t(a);
BEGIN;
INSERT INTO t VALUES (1);
SAVEPOINT s0;
CREATE VIRTUAL TABLE ft USING fts5(x);
INSERT INTO ft VALUES ('apple');
ROLLBACK TO s0;
SAVEPOINT s1;
COMMIT;
SELECT count(*) FROM t;
SELECT 'q2';
BEGIN;
INSERT INTO t VALUES (2);
SAVEPOINT s0;
CREATE VIRTUAL TABLE ft4 USING fts4(x);
INSERT INTO ft4 VALUES ('apple');
ROLLBACK TO s0;
COMMIT;
SELECT count(*) FROM t;
SELECT 'q3';
SELECT name FROM sqlite_schema ORDER BY name;
