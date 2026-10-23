-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5-secure-delete-dlidx-walks-next-term
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected":
--   q1|
--   q2|10
--   q3|
--   q4|30
-- Buggy builds print "q1|58,59,60", "q2|7", "q3|159,160,...,170" (12 rowids) and "q4|18"
-- (trunk 185ffd27064b, 3.53.2, 3.51.0, 3.50.2, 3.46.0; also with the fixes for the
-- related secure-delete and tokendata=1 reports applied).
-- q1/q2: detail=full, last rowid of 'a' removed. q3/q4: detail=none, the last 35
-- rowids of 'a' removed, so the page the doclist-index points to is empty.
CREATE VIRTUAL TABLE t USING fts5(x);
INSERT INTO t(t, rank) VALUES('pgsz', 32);
WITH s(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM s WHERE i<60)
  INSERT INTO t(rowid, x) SELECT i, iif(i<=50, 'a', 'b') FROM s;
INSERT INTO t(t, rank) VALUES('secure-delete', 1);
DELETE FROM t WHERE rowid = 50;
SELECT 'q1', coalesce(group_concat(rowid, ','), '') FROM (SELECT rowid FROM t('a') WHERE rowid >= 51 ORDER BY rowid);
SELECT 'q2', count(*) FROM t('b NOT a');
CREATE VIRTUAL TABLE u USING fts5(x, detail=none);
INSERT INTO u(u, rank) VALUES('pgsz', 32);
WITH s(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM s WHERE i<170)
  INSERT INTO u(rowid, x) SELECT i, iif(i<=140, 'a', 'b') FROM s;
INSERT INTO u(u, rank) VALUES('secure-delete', 1);
DELETE FROM u WHERE rowid > 105 AND rowid <= 140;
SELECT 'q3', coalesce(group_concat(rowid, ','), '') FROM (SELECT rowid FROM u('a') WHERE rowid >= 110 ORDER BY rowid);
SELECT 'q4', count(*) FROM u('b NOT a');
