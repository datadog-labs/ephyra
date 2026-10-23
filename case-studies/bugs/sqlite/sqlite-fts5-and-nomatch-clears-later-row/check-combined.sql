-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5-and-nomatch-clears-later-row,
-- with fix-combined.diff, which also fixes ../sqlite-fts5-not-eof-rhs-instances.
-- Run as: sqlite3 -batch -init /dev/null :memory: < check-combined.sql (a build with FTS5)
-- q1, q2: check.sql of this folder (AND bug); q3, q4: ../sqlite-fts5-not-eof-rhs-instances/check.sql
-- (NOT-EOF bug), renumbered. Its output on a correct build (fix-combined.diff applied) is the
-- registry's "expected"; trunk printing exactly that means both bugs were fixed upstream:
--   q1|[x] [z]
--   q2|[x] [z]
--   q3|[g] b
--   q4|[g] b
-- Buggy builds (trunk 3769f60be38b, 3.46.0 to 3.53.2) print
--   q1|[x] z
--   q2|[x] z
--   q3|[g] [b]
--   q4|[g] [b]
-- A build with only one of the two bugs fixed gets one pair right.
CREATE VIRTUAL TABLE t1 USING fts5(a);
INSERT INTO t1(rowid, a) VALUES (1, 'x y'), (2, 'x z');
SELECT 'q1', highlight(t1, 0, '[', ']') FROM t1('(x AND (z OR "y x")) OR zz');
CREATE VIRTUAL TABLE t2 USING fts5(a, b, detail=column);
INSERT INTO t2(rowid, a, b) VALUES (1, 'x y', ''), (2, 'x z', '');
SELECT 'q2', highlight(t2, 0, '[', ']') FROM t2('(x AND (z OR b : y)) OR zz');
CREATE VIRTUAL TABLE t3 USING fts5(a);
INSERT INTO t3(rowid, a) VALUES (1, 'g b');
SELECT 'q3', highlight(t3, 0, '[', ']') FROM t3('g OR (zz NOT b)');
CREATE VIRTUAL TABLE t4 USING fts5(a, detail=none);
INSERT INTO t4(rowid, a) VALUES (1, 'g b');
SELECT 'q4', highlight(t4, 0, '[', ']') FROM t4('g OR (zz NOT b)');
