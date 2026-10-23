-- A row-value range on a WITHOUT ROWID table's primary key that runs on into a
-- column added by ALTER TABLE ADD COLUMN. The planner seeks the PRIMARY KEY
-- b-tree with a key that includes the added column (whereRangeVectorLen() caps
-- the vector at nColumn, not nKeyCol); rows written before the ALTER have no
-- field for it, and the record comparator treats the short record as corrupt.
CREATE TABLE t1(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t1 VALUES(1.5), (2.5);
ALTER TABLE t1 ADD COLUMN b DEFAULT 5;
PRAGMA integrity_check;
SELECT * FROM t1 WHERE (a, b) > (1.5, 0);
-- malformed       expected 1.5|5  2.5|5
SELECT * FROM t1 WHERE (a, b) < (2.5, 9);
-- 1.5|5           expected 1.5|5  2.5|5

-- The form the fuzzer found: the RIGHT JOIN lets the ON term drive the seek on
-- t1; the LEFT JOIN with the tables swapped cannot, and is right.
CREATE TABLE t2(x);
INSERT INTO t2 VALUES(0);
SELECT * FROM t2 RIGHT JOIN t1 ON (t1.a, t1.b) > (1.5, 0);
-- malformed       expected 0|1.5|5  0|2.5|5
SELECT * FROM t1 LEFT JOIN t2 ON (t1.a, t1.b) > (1.5, 0);
-- 1.5|5|0  2.5|5|0
