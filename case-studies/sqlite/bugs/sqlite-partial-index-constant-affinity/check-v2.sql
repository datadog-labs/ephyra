-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-partial-index-constant-affinity
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t1(a, b TEXT);
INSERT INTO t1 VALUES(1, '5.0');
CREATE INDEX t1a ON t1(a) WHERE b = CAST(5 AS INT);
SELECT 'q1';
SELECT b FROM t1 WHERE b = CAST(5 AS INT);
SELECT 'q2';
SELECT count(*) FROM t1 WHERE b = CAST(5 AS INT) AND b LIKE '%.0';
CREATE TABLE t2(a, b INT);
INSERT INTO t2 VALUES(1, -9223372036854775808);
CREATE INDEX t2a ON t2(a) WHERE b = -9223372036854775808.0;
SELECT 'q3';
SELECT typeof(b) FROM t2 WHERE b = -9223372036854775808.0;
CREATE TABLE t3(a, b INT);
INSERT INTO t3 VALUES(1, '-9223372036854775808.0');
CREATE INDEX t3a ON t3(a) WHERE b = -9223372036854775808;
SELECT 'q4';
SELECT typeof(b) FROM t3 WHERE b = -9223372036854775808;
CREATE TABLE t4(a ANY, b INT) STRICT;
INSERT INTO t4 VALUES(1, -9223372036854775808);
CREATE INDEX t4a ON t4(a) WHERE b = -9.223372036854776e18;
SELECT 'q5';
SELECT typeof(b) FROM t4 WHERE b = -9.223372036854776e18;
