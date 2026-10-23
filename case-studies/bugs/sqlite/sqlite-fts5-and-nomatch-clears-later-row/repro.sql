-- sqlite-fts5-and-nomatch-clears-later-row: run with sqlite3 -batch -init /dev/null :memory: < repro.sql
-- Expected (fixed build): four times
--   2|[x] [z]
-- Trunk 3769f60be38b, 3.51.0 (and 3.46.0, 3.50.2, 3.53.2): the first line is right, the other three are
--   2|[x] z
CREATE VIRTUAL TABLE t1 USING fts5(a);
INSERT INTO t1(rowid, a) VALUES (1, 'x y'), (2, 'x z');
SELECT rowid, highlight(t1, 0, '[', ']') FROM t1('x AND (z OR "y x")');
SELECT rowid, highlight(t1, 0, '[', ']') FROM t1('(x AND (z OR "y x")) OR zz');
SELECT rowid, highlight(t1, 0, '[', ']') FROM t1('(x AND (z OR "y x")) NOT zz');
CREATE VIRTUAL TABLE t2 USING fts5(a, b, detail=column);
INSERT INTO t2(rowid, a, b) VALUES (1, 'x y', ''), (2, 'x z', '');
SELECT rowid, highlight(t2, 0, '[', ']') FROM t2('(x AND (z OR b : y)) OR zz');
