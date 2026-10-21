-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-rtree-update-set-rowid-ignored
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with R-Tree)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
CREATE VIRTUAL TABLE r USING rtree(id, x1, x2, +aux);
INSERT INTO r VALUES (1, 0, 1, 'a'), (2, 2, 3, 'b');
UPDATE r SET rowid = 9 WHERE id = 2;
SELECT id, x1, x2, aux FROM r ORDER BY id;
SELECT 'q2';
UPDATE r SET _rowid_ = 8, x2 = 7 WHERE id = 1;
SELECT id, x1, x2, aux FROM r ORDER BY id;
SELECT 'q3';
UPDATE r SET oid = oid + 100;
SELECT group_concat(id) FROM (SELECT id FROM r WHERE x1<=2.5 AND x2>=2.5 ORDER BY id);
SELECT 'q4';
UPDATE r SET id = 6 WHERE id = 108;
SELECT group_concat(id) FROM (SELECT id FROM r ORDER BY id);
