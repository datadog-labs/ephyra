-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5-secure-delete-merge-drops-marker
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected":
--   q1|
--   q2|
--   q3|
-- Buggy builds print "q1|1", "q2|1" and "q3|1" (trunk 185ffd27064b, 3.53.2, 3.51.0, 3.46.0;
-- also with only the detail=none secure-delete fix applied).
-- detail=full and detail=column only: detail=none also needs the detail=none fix.
CREATE VIRTUAL TABLE t USING fts5(x, detail=full);
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
SELECT 'q1', coalesce(group_concat(rowid, ','), '') FROM (SELECT rowid FROM t WHERE t MATCH 'c' ORDER BY rowid);
CREATE VIRTUAL TABLE u USING fts5(x, detail=column);
INSERT INTO u(u, rank) VALUES ('pgsz', 32);
INSERT INTO u(u, rank) VALUES ('automerge', 0);
INSERT INTO u(u, rank) VALUES ('crisismerge', 3);
INSERT INTO u(rowid, x) VALUES (1, 'c');
INSERT INTO u(rowid, x) VALUES (2, 'a b d e f g h');
INSERT INTO u(u) VALUES ('optimize');
INSERT OR REPLACE INTO u(rowid, x) VALUES (1, 'c');
DELETE FROM u WHERE rowid = 1;
INSERT INTO u(u, rank) VALUES ('secure-delete', 1);
INSERT INTO u(rowid, x) VALUES (1, 'c');
UPDATE u SET x = 'b' WHERE rowid = 1;
SELECT 'q2', coalesce(group_concat(rowid, ','), '') FROM (SELECT rowid FROM u WHERE u MATCH 'c' ORDER BY rowid);
CREATE VIRTUAL TABLE v USING fts5(x, detail=full);
INSERT INTO v(v, rank) VALUES ('pgsz', 32);
INSERT INTO v(rowid, x) VALUES (1, 'c');
INSERT INTO v(rowid, x) VALUES (2, 'm n o p q r s t u v w x y z');
INSERT INTO v(v) VALUES ('optimize');
DELETE FROM v WHERE rowid = 1;
INSERT INTO v(v, rank) VALUES ('secure-delete', 1);
INSERT INTO v(rowid, x) VALUES (1, 'c'), (3, 'aa ab ac ad ae af ag ah');
INSERT INTO v(v, rank) VALUES ('usermerge', 2);
INSERT INTO v(v, rank) VALUES ('merge', 1);
DELETE FROM v WHERE rowid = 1;
SELECT 'q3', coalesce(group_concat(rowid, ','), '') FROM (SELECT rowid FROM v WHERE v MATCH 'c' ORDER BY rowid);
