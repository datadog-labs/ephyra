-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-geopoly-rowid-eq-nonint
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with Geopoly)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
CREATE VIRTUAL TABLE g USING geopoly();
INSERT INTO g(rowid, _shape) VALUES
  (6, '[[0,0],[1,0],[0,1],[0,0]]'), (7, '[[0,0],[1,0],[0,1],[0,0]]');
SELECT count(*) FROM g WHERE rowid = 6.5;
SELECT 'q2';
SELECT count(*) FROM g WHERE rowid = '6abc';
SELECT 'q3';
SELECT group_concat(rowid) FROM g WHERE rowid = 6.0;
SELECT 'q4';
SELECT count(*) FROM g WHERE rowid IN (6.5, 'x', 1e19, 7);
SELECT 'q5';
CREATE TABLE t1(x);
INSERT INTO t1 VALUES (6), (6.5), ('7abc');
SELECT t1.x, g.rowid FROM t1 LEFT JOIN g ON g.rowid = t1.x;
SELECT 'q6';
DELETE FROM g WHERE rowid = 7.5;
SELECT group_concat(rowid) FROM g;
