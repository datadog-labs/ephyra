-- A Bloom filter in front of an INTEGER PRIMARY KEY lookup drops a join key
-- that is the REAL -9223372036854775808.0 (exactly -2^63), although that value
-- equals the rowid -9223372036854775808. The plain lookup finds the row since
-- check-in 987ba5a3dc5f2a22 (forum post 2026-07-19T07:31:46Z, in 3.54.0), but
-- the Bloom filter path codes OP_MustBeInt before OP_Filter, and OP_MustBeInt
-- still rejects -2^63 as a REAL.
-- Tested on trunk dc388e83b05d (2026-10-10) and 3.54.0.

CREATE TABLE t1(a);
INSERT INTO t1 VALUES(-9223372036854775808.0);
CREATE TABLE t2(b INTEGER PRIMARY KEY);
INSERT INTO t2 VALUES(-9223372036854775808);
ANALYZE;
-- Make the outer table look larger than t2, so that the planner adds the
-- Bloom filter (more rows give the same plan).
UPDATE sqlite_stat1 SET stat='100' WHERE tbl='t1';
UPDATE sqlite_stat1 SET stat='10' WHERE tbl='t2';
ANALYZE sqlite_schema;

SELECT a = -9223372036854775808 FROM t1;
-- 1   (INTEGER and REAL compare numerically, and exactly)

SELECT count(*) FROM t1, t2 WHERE b = a;
-- 1   (SEARCH t2 USING INTEGER PRIMARY KEY, no Bloom filter)

SELECT count(*) FROM t1, t2 WHERE b = a AND b <> 5;
-- expected: 1
-- actual:   0   (BLOOM FILTER ON t2 (b=?) + SEARCH t2 USING INTEGER PRIMARY KEY)

-- Right with ".testctrl optimizations -BloomFilter" or with fix.diff.

-- The same OP_MustBeInt makes a foreign key check on an INTEGER PRIMARY KEY
-- parent fail for this child value (wrong in every version we tried, 3.28.0
-- to trunk):
PRAGMA foreign_keys = ON;
CREATE TABLE p(id INTEGER PRIMARY KEY);
INSERT INTO p VALUES(-9223372036854775808);
CREATE TABLE c(r REFERENCES p);
INSERT INTO c VALUES(-9223372036854775808.0);
-- expected: succeeds (the parent key equals the child key)
-- actual:   FOREIGN KEY constraint failed
SELECT count(*) FROM c JOIN p ON p.id = c.r;
-- 1 with fix.diff (0 rows in c otherwise)
