-- sqlite-fts5-not-eof-rhs-instances: run with sqlite3 -batch -init /dev/null :memory: < repro.sql
-- Expected (fixed build):
--   1|[g] b
--   1|[g] b
--   1|[g] b
--   2|[zz]
-- Trunk 3769f60be38b, 3.51.0 (and 3.46.0, 3.50.2, 3.53.2): the first line is
--   1|[g] [b]
-- The second query (AND instead of NOT) and the third (same query after a row containing zz is added) are right.
CREATE VIRTUAL TABLE t1 USING fts5(a);
INSERT INTO t1(rowid, a) VALUES (1, 'g b');
SELECT rowid, highlight(t1, 0, '[', ']') FROM t1('g OR (zz NOT b)');
SELECT rowid, highlight(t1, 0, '[', ']') FROM t1('g OR (zz AND b)');
INSERT INTO t1(rowid, a) VALUES (2, 'zz');
SELECT rowid, highlight(t1, 0, '[', ']') FROM t1('g OR (zz NOT b)');
