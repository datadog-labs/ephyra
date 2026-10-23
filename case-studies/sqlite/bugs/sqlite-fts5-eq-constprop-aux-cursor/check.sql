-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5-eq-constprop-aux-cursor
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
CREATE VIRTUAL TABLE ft USING fts5(a);
INSERT INTO ft(rowid, a) VALUES (1, 'one apple'), (2, 'apple pie');
SELECT rowid FROM ft WHERE ft = 'apple' AND snippet(ft, 0, '[', ']', '..', 4) IS NOT NULL;
SELECT 'q2';
SELECT rowid FROM ft WHERE ft = 'apple' AND bm25(ft) < 0;
SELECT 'q3';
SELECT count(*) FROM ft WHERE ft = 'apple' AND ft = 'pie';
SELECT 'q4';
SELECT count(*) FROM ft WHERE ft MATCH 'apple' AND rank = 'bm25(10.0)' AND rank < 0;
