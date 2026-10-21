-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-gencol-index-real-int
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t(a REAL, b AS (a) VIRTUAL, c);
INSERT INTO t(a, c) VALUES(1.0, 'x'), (2.5, 'y'), (3.0, 'z');
CREATE INDEX i ON t(b);
CREATE TABLE u(a REAL, b BLOB AS (a), c AS (b));
INSERT INTO u(a) VALUES(4.0);
CREATE INDEX ub ON u(b);
CREATE INDEX uc ON u(c);
SELECT 'q1';
SELECT quote(b), typeof(b) FROM t INDEXED BY i ORDER BY b;
SELECT 'q2';
SELECT quote(b), c FROM t WHERE b = 1;
SELECT 'q3';
SELECT count(*) FROM t INDEXED BY i WHERE typeof(b) = 'real';
SELECT 'q4';
SELECT quote(b / 2) FROM t INDEXED BY i WHERE b > 2.9;
SELECT 'q5';
SELECT quote(b), typeof(b) FROM u INDEXED BY ub;
SELECT 'q6';
SELECT quote(c), typeof(c) FROM u INDEXED BY uc;
SELECT 'q7';
REINDEX i;
SELECT group_concat(quote(b), ' ') FROM t INDEXED BY i;
