-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5-match-rowid-eq-nonint
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
CREATE VIRTUAL TABLE ft USING fts5(a);
INSERT INTO ft(rowid, a) VALUES (1, 'apple'), (2, 'apple'), (3, 'apple pie');
SELECT count(*) FROM ft WHERE ft MATCH 'apple' AND rowid = 2.5;
SELECT 'q2';
SELECT count(*) FROM ft WHERE ft MATCH 'apple' AND rowid = 'x';
SELECT 'q3';
SELECT count(*) FROM ft WHERE ft MATCH 'apple' AND rowid = NULL;
SELECT 'q4';
SELECT group_concat(rowid) FROM ft WHERE ft MATCH 'apple' AND rowid = 2.0;
SELECT 'q5';
CREATE TABLE t1(x);
INSERT INTO t1 VALUES (3), (2.5), ('x');
SELECT t1.x, ft.rowid FROM t1 LEFT JOIN ft ON ft.rowid = t1.x AND ft MATCH 'pie';
SELECT 'q6';
SELECT count(*) FROM ft WHERE ft MATCH 'apple' AND rowid IN (2.5, 'x', 3);
