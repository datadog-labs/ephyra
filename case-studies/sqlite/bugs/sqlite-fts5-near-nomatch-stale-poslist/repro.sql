-- sqlite-fts5-near-nomatch-stale-poslist: run with sqlite3 -batch -init /dev/null :memory: < repro.sql
-- Expected (fixed build):
--   1|[h] e d [c]
--   2|p q s t u
--   1|[h] e d..
--   2|x
--   [r] h d c
-- Trunk 3769f60be38b and 3.51.0: "2|p q s [t] u", then "database disk image is malformed" at the
-- snippet() query (after printing 1|[h] e d..), then "[r] [h] d c".
CREATE VIRTUAL TABLE t1 USING fts5(a, b);
INSERT INTO t1(rowid, a, b) VALUES (1, 'one', 'h e d c');
INSERT INTO t1(rowid, a, b) VALUES (2, 'r h c', 'p q s t u');
SELECT rowid, highlight(t1, 1, '[', ']') FROM t1('b : NEAR(h c, 2) OR r');
UPDATE t1 SET b = 'x' WHERE rowid = 2;
SELECT rowid, snippet(t1, 1, '[', ']', '..', 3) FROM t1('b : NEAR(h c, 2) OR r');
CREATE VIRTUAL TABLE t2 USING fts5(a);
INSERT INTO t2(a) VALUES ('r h d c');
SELECT highlight(t2, 0, '[', ']') FROM t2('NEAR(h "c d") OR r');
