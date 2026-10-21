-- FTS5 secure-delete: after the last rowids of a long doclist are removed,
-- a query with a rowid lower bound returns rowids from the next term.
-- Run as: sqlite3 :memory: < repro.sql
--
-- Buggy (trunk 185ffd27064b, 3.53.2, 3.51.0, 3.50.2, 3.46.0):
--   58
--   59
--   60
--   7
-- Expected (fixed build; also what the same statements give without the
-- secure-delete line, or with 'rebuild' after the DELETE):
--   10
-- Rows 51 to 60 contain only 'b', and no row with rowid >= 51 contains 'a'.
-- integrity-check passes on both.
CREATE VIRTUAL TABLE t USING fts5(x);
INSERT INTO t(t, rank) VALUES('pgsz', 32);
WITH s(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM s WHERE i<60)
  INSERT INTO t(rowid, x) SELECT i, iif(i<=50, 'a', 'b') FROM s;
INSERT INTO t(t, rank) VALUES('secure-delete', 1);
DELETE FROM t WHERE rowid = 50;
SELECT rowid FROM t('a') WHERE rowid >= 51;
SELECT count(*) FROM t('b NOT a');
INSERT INTO t(t) VALUES('integrity-check');
