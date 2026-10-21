-- FTS5 detail=none: secure-delete of a rowid that was replaced while
-- secure-delete was off damages the index.
-- Run as: sqlite3 :memory: < repro.sql
--
-- Buggy (trunk 185ffd27064b, 3.51.0):
--   0
--   1
--   5
--   6
--   Error near line ...: fts5: checksum mismatch for table "t"
-- Expected (fixed build; also what detail=full/column, or the same
-- statements with secure-delete on from the start, or a 'rebuild' before
-- turning it on, give): a single line "0" and no error.
CREATE VIRTUAL TABLE t USING fts5(x, detail=none);
INSERT INTO t(rowid, x) VALUES (1, 'a');
INSERT OR REPLACE INTO t(rowid, x) VALUES (1, 'a');
INSERT INTO t(t, rank) VALUES ('secure-delete', 1);
DELETE FROM t WHERE rowid = 1;
SELECT count(*) FROM t;
SELECT rowid FROM t WHERE t MATCH 'a';
INSERT INTO t(t) VALUES ('integrity-check');
