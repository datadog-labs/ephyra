-- Pre-triage check: sqlite-fts5-tokendata-prefix-corrupt
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Correct build (fix.diff applied) prints q1 1,2  q2 (empty)  q3 4  q4 1,2.
-- Buggy builds (3.46.0 .. 3.53.2 amalgamations, 3.51.0, trunk 3769f60be38b,
-- 03f699544e6b) print an SQLITE_CORRUPT error for q1, q2 and q3 ("database
-- disk image is malformed" or "fts5: corruption found reading blob ...");
-- the error and its text depend on freed memory. q4 (term first) is right.
CREATE VIRTUAL TABLE t1 USING fts5(x, tokendata=1, prefix=1);
CREATE VIRTUAL TABLE t2 USING fts5(x, tokendata=1, prefix=1);
CREATE VIRTUAL TABLE t3 USING fts5(x, tokendata=1, prefix=1);
CREATE VIRTUAL TABLE t4 USING fts5(x, tokendata=1, prefix=1);
INSERT INTO t1 VALUES('vv aa');
INSERT INTO t2 VALUES('vv aa');
INSERT INTO t3 VALUES('vv aa');
INSERT INTO t4 VALUES('vv aa');
BEGIN;
INSERT INTO t1 VALUES('vv aa');
INSERT INTO t2 VALUES('vv aa');
INSERT INTO t3 VALUES('vv aa');
INSERT INTO t4 VALUES('vv aa');
SELECT 'q1';
SELECT group_concat(rowid) FROM t1 WHERE t1 MATCH 'v* AND aa';
SELECT 'q2';
SELECT group_concat(rowid) FROM t2 WHERE t2 MATCH 'v* NOT aa';
SELECT 'q3';
SELECT count(*) FROM t3 AS a, t3 AS b WHERE a.t3 MATCH 'v*' AND b.t3 MATCH 'aa';
SELECT 'q4';
SELECT group_concat(rowid) FROM t4 WHERE t4 MATCH 'aa AND v*';
COMMIT;
