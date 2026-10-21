-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5-contentless-unindexed-drop-rename
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
CREATE VIRTUAL TABLE h USING fts5(a, b UNINDEXED, content='', contentless_unindexed=1);
INSERT INTO h(rowid, a, b) VALUES (1, 'x y', 'b1');
ALTER TABLE h RENAME TO h2;
SELECT name FROM sqlite_schema WHERE name LIKE 'h\_%' ESCAPE '\';
SELECT 'q2';
INSERT INTO h2(rowid, a, b) VALUES (2, 'x z', 'b2');
SELECT rowid, b FROM h2('x');
SELECT 'q3';
DROP TABLE h2;
SELECT name FROM sqlite_schema;
SELECT 'q4';
CREATE VIRTUAL TABLE h3 USING fts5(a, b UNINDEXED, content='', contentless_unindexed=1, contentless_delete=1);
DROP TABLE h3;
SELECT count(*) FROM sqlite_schema;
