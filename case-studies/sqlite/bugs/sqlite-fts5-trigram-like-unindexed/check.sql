-- Pre-triage check: sqlite-fts5-trigram-like-unindexed
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Correct build (fix.diff applied) prints q1 1, q2 1, q3 1, q4 1.
-- Buggy builds (3.46.0 .. 3.53.2 amalgamations, 3.51.0, trunk 3769f60be38b)
-- print 0 for q1, q2 and q4 (q3, without the pushdown, prints 1).
CREATE VIRTUAL TABLE t1 USING fts5(a, b UNINDEXED, tokenize=trigram);
INSERT INTO t1 VALUES('xyz', 'abcdef');
SELECT 'q1';
SELECT count(*) FROM t1 WHERE b LIKE '%bcd%';
SELECT 'q2';
SELECT count(*) FROM t1 WHERE b GLOB '*bcd*';
SELECT 'q3';
SELECT count(*) FROM t1 WHERE +b LIKE '%bcd%';
CREATE VIRTUAL TABLE t2 USING fts5(s0, s1 UNINDEXED, tokenize=trigram, locale=1);
INSERT INTO t2(rowid, s0, s1) VALUES(1, 'abc', fts5_locale('', 'CONDITIONAL'));
SELECT 'q4';
SELECT count(*) FROM t2 a0 CROSS JOIN t2 a1 WHERE a0.s1 GLOB 'CONDITIO*NAL*';
