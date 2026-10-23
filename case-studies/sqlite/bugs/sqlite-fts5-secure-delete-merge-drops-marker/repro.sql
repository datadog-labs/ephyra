-- FTS5: with secure-delete on, a merge drops the delete marker that hides
-- an older entry, and a later secure-delete then exposes that entry.
-- Run as: sqlite3 :memory: < repro.sql
--
-- Buggy (trunk 185ffd27064b, 3.53.2, 3.51.0, 3.50.2, 3.46.0):
--   1
--   1
--   Error near line ...: fts5: checksum mismatch for table "t"
--   (3.51.0 and older: "database disk image is malformed")
-- Expected (fixed build; also what the same statements give without the
-- 'merge' line, with secure-delete on from the start, or with 'rebuild'
-- instead of 'optimize'):
--   1
-- The table holds only row 2, which does not contain 'c'. Same result with
-- detail=column and detail=none (detail=none needs the fix for the
-- detail=none secure-delete report as well).
CREATE VIRTUAL TABLE t USING fts5(x);
INSERT INTO t(t, rank) VALUES ('pgsz', 32);
INSERT INTO t(rowid, x) VALUES (1, 'c');
INSERT INTO t(rowid, x) VALUES (2, 'a b d e f g h');
INSERT INTO t(t) VALUES ('optimize');
DELETE FROM t WHERE rowid = 1;
INSERT INTO t(t, rank) VALUES ('secure-delete', 1);
INSERT INTO t(rowid, x) VALUES (1, 'c');
INSERT INTO t(t, rank) VALUES ('usermerge', 2);
INSERT INTO t(t, rank) VALUES ('merge', 1);
DELETE FROM t WHERE rowid = 1;
SELECT count(*) FROM t;
SELECT rowid FROM t WHERE t MATCH 'c';
INSERT INTO t(t) VALUES ('integrity-check');
