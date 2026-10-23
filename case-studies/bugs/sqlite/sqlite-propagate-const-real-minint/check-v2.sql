-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-propagate-const-real-minint
-- Run as: sqlite3 -batch -init /dev/null :memory: < check-v2.sql
-- v2 (2026-10-21) of check.sql: upstream 03f699544e6b now stores the REAL
-- literal -2^63 as an INTEGER under INT affinity, so check.sql's q3 no longer
-- set up a stored REAL -2^63; here q3 inserts the text '-9223372036854775808.0',
-- which every build stores as a REAL. Correct output: q1 1, q2 1, q3 1, q4 1
-- (fix-trunk.diff applied; also the output with `.testctrl optimizations
-- -PropagateConst`). 3769f60be38b prints 0 0 0 0; 03f699544e6b 1 1 0 1 (q1, q2
-- and q4 fixed upstream by the bFromText change, the stored-REAL direction not).
CREATE TABLE t(a, b INT);
INSERT INTO t VALUES(1, -9223372036854775808);
SELECT 'q1';
SELECT count(*) FROM t WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer';
SELECT 'q2';
SELECT count(*) FROM t WHERE b = -9223372036854775808.0 AND b || '' = '-9223372036854775808';
CREATE TABLE u(a, b INT);
INSERT INTO u VALUES(1, '-9223372036854775808.0');
SELECT 'q3';
SELECT count(*) FROM u WHERE b = -9223372036854775808 AND typeof(b) = 'real';
CREATE TABLE o(x);
INSERT INTO o VALUES(1);
SELECT 'q4';
SELECT count(*) FROM o CROSS JOIN t WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer';
