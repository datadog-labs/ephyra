-- Pre-triage check: sqlite-update-real-minint-stale-index
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Correct build (fix.diff applied; also 3769f60be38b and 3.54.0) prints:
--   q1|real|real
--   q2|real|real
-- Buggy build (trunk 03f699544e6b) prints:
--   q1|integer|real
--   q2|integer|real
CREATE TABLE t1(a INT, b);
CREATE INDEX t1a ON t1(a);
INSERT INTO t1 VALUES('-9223372036854775809', 1);
UPDATE t1 SET b = 2;
SELECT 'q1', (SELECT typeof(a) FROM t1 NOT INDEXED),
  (SELECT typeof(a) FROM t1 INDEXED BY t1a WHERE a < 0);
CREATE TABLE t2(a INT, b, c, PRIMARY KEY(c, a)) WITHOUT ROWID;
CREATE INDEX t2c ON t2(c);
INSERT INTO t2 VALUES('-9223372036854775809', 1, 5);
UPDATE t2 SET b = 2;
SELECT 'q2', (SELECT typeof(a) FROM t2 NOT INDEXED),
  (SELECT typeof(a) FROM t2 INDEXED BY t2c WHERE c = 5);
