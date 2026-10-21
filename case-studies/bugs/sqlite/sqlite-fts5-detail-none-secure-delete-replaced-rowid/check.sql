-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5-detail-none-secure-delete-replaced-rowid
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected":
--   q1|
--   q2|1,2,4,5
-- Buggy builds print "q1|1,5,6" and "q2|1,2,3,4,5,7,8" (trunk 185ffd27064b, 3.51.0).
CREATE VIRTUAL TABLE t USING fts5(x, detail=none);
INSERT INTO t(rowid, x) VALUES (1, 'a');
INSERT OR REPLACE INTO t(rowid, x) VALUES (1, 'a');
INSERT INTO t(t, rank) VALUES ('secure-delete', 1);
DELETE FROM t WHERE rowid = 1;
SELECT 'q1', coalesce(group_concat(rowid, ','), '') FROM (SELECT rowid FROM t WHERE t MATCH 'a' ORDER BY rowid);
CREATE VIRTUAL TABLE u USING fts5(x, detail=none);
INSERT INTO u(rowid, x) VALUES (1, 'a'), (2, 'a'), (3, 'a'), (4, 'a'), (5, 'a');
INSERT OR REPLACE INTO u(rowid, x) VALUES (3, 'a');
INSERT INTO u(u, rank) VALUES ('secure-delete', 1);
DELETE FROM u WHERE rowid = 3;
SELECT 'q2', coalesce(group_concat(rowid, ','), '') FROM (SELECT rowid FROM u WHERE u MATCH 'a' ORDER BY rowid);
