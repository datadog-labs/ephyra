-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-propagate-const-real-minint
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build is the registry's "expected" (no fix: the
-- output with constant propagation off, `.testctrl optimizations
-- -PropagateConst`); trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t(a, b INT);
INSERT INTO t VALUES(1, -9223372036854775808);
SELECT 'q1';
SELECT count(*) FROM t WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer';
SELECT 'q2';
SELECT count(*) FROM t WHERE b = -9223372036854775808.0 AND b || '' = '-9223372036854775808';
CREATE TABLE u(a, b INT);
INSERT INTO u VALUES(1, -9223372036854775808.0);
SELECT 'q3';
SELECT count(*) FROM u WHERE b = -9223372036854775808 AND typeof(b) = 'real';
CREATE TABLE o(x);
INSERT INTO o VALUES(1);
SELECT 'q4';
SELECT count(*) FROM o CROSS JOIN t WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer';
