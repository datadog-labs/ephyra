-- Pre-triage check for sqlite-fts5-and-nomatch-clears-later-row
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected":
--   q1|[x] [z]
--   q2|[x] [z]
-- Buggy builds (trunk 3769f60be38b, 3.46.0 to 3.53.2) print
--   q1|[x] z
--   q2|[x] z
CREATE VIRTUAL TABLE t1 USING fts5(a);
INSERT INTO t1(rowid, a) VALUES (1, 'x y'), (2, 'x z');
SELECT 'q1', highlight(t1, 0, '[', ']') FROM t1('(x AND (z OR "y x")) OR zz');
CREATE VIRTUAL TABLE t2 USING fts5(a, b, detail=column);
INSERT INTO t2(rowid, a, b) VALUES (1, 'x y', ''), (2, 'x z', '');
SELECT 'q2', highlight(t2, 0, '[', ']') FROM t2('(x AND (z OR b : y)) OR zz');
