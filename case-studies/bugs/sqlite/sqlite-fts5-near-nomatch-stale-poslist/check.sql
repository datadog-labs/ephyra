-- Pre-triage check for sqlite-fts5-near-nomatch-stale-poslist
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected":
--   q1|1:[h] e d [c],2:p q s t u
--   q2|[r] h d c
-- Buggy builds (trunk 3769f60be38b, 3.46.0 to 3.53.2) print
--   q1|1:[h] e d [c],2:p q s [t] u
--   q2|[r] [h] d c
CREATE VIRTUAL TABLE t1 USING fts5(a, b);
INSERT INTO t1(rowid, a, b) VALUES (1, 'one', 'h e d c');
INSERT INTO t1(rowid, a, b) VALUES (2, 'r h c', 'p q s t u');
SELECT 'q1', group_concat(rowid || ':' || h, ',') FROM (SELECT rowid, highlight(t1, 1, '[', ']') AS h FROM t1('b : NEAR(h c, 2) OR r') ORDER BY rowid);
CREATE VIRTUAL TABLE t2 USING fts5(a);
INSERT INTO t2(a) VALUES ('r h d c');
SELECT 'q2', highlight(t2, 0, '[', ']') FROM t2('NEAR(h "c d") OR r');
