-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5vocab-rowid-or-right-join
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
-- Buggy builds print "q1|2|1|banana" (banana twice, apple lost) and
-- "q2|banana cherry" (apple lost, cherry reported unmatched).
-- Fixed builds print "q1|2|2|apple" and "q2|apple banana".
CREATE VIRTUAL TABLE ft USING fts5(x);
INSERT INTO ft VALUES('apple banana cherry');
CREATE VIRTUAL TABLE v USING fts5vocab(ft, 'row');
CREATE TABLE t(a);
INSERT INTO t VALUES('cherry');
SELECT 'q1', count(*), count(DISTINCT term), min(term)
  FROM v WHERE term = 'banana' OR term < 'c';
SELECT 'q2', group_concat(term, ' ')
  FROM (SELECT v.term FROM t RIGHT JOIN v ON v.term = t.a WHERE t.a IS NULL);
