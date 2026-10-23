-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-geopoly-update-rowid-nochange
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with Geopoly)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
CREATE VIRTUAL TABLE g USING geopoly(a);
INSERT INTO g(rowid, _shape, a) VALUES
  (1, '[[0,0],[1,0],[0,1],[0,0]]', 'x'), (2, '[[5,5],[6,5],[5,6],[5,5]]', 'y');
UPDATE g SET rowid = 7 WHERE rowid = 1;
SELECT rowid, a, geopoly_json(_shape) FROM g ORDER BY rowid;
SELECT 'q2';
SELECT rowid FROM g WHERE geopoly_overlap(_shape, '[[0,0],[1,0],[0,1],[0,0]]');
SELECT 'q3';
UPDATE g SET rowid = rowid + 10, a = a || '!';
SELECT rowid, a FROM g ORDER BY rowid;
SELECT 'q4';
PRAGMA integrity_check;
