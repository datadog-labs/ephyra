-- CLUSTER (index-scan path) drops a live row that the current transaction
-- inserted and then HOT-updated inside a rolled-back subtransaction.
-- Independent of autovacuum / planner stats: the index exists before any
-- rows, so reltuples = -1 and CLUSTER picks the index scan.
-- Expected: 1 1.  Actual (14.24, 18.6, 19 RC1 code, master 158b6d5109): 0 0.
CREATE TABLE t1(c0 int, c1 int);
CREATE INDEX t1_i0 ON t1 (c1);
BEGIN;
INSERT INTO t1 VALUES (1, 1);
SAVEPOINT s1; UPDATE t1 SET c0 = 2; ROLLBACK TO s1;
CLUSTER t1 USING t1_i0;
SELECT count(*) FROM t1;
COMMIT;
SELECT count(*) FROM t1;
