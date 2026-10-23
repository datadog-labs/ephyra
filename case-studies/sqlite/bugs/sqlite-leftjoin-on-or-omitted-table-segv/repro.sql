-- An OR in the ON clause of a LEFT JOIN, where every OR branch names a
-- column of a table to the left of the join, is planned as a MULTI-INDEX OR
-- loop on that left table (whereLoopAddOr does not check EP_OuterON). When the
-- LEFT JOIN table is then dropped as unused (DISTINCT, omit-noop-join), the
-- OR branches are coded as filters on the left table, and a branch that reads
-- the dropped table reads a cursor that was never opened.
CREATE TABLE t1(a, b);
CREATE INDEX t1b ON t1(b);
CREATE TABLE t2(c);
INSERT INTO t1 VALUES(1,1),(2,1),(3,1),(4,1),(5,1),(6,1);
ANALYZE;

-- The ON clause filters the left table:
SELECT DISTINCT y.a FROM t1 AS x JOIN t1 AS y ON y.b >= x.b AND y.b <= x.b
  LEFT JOIN t2 ON y.a = 5 OR y.a = 6
 WHERE y.b BETWEEN 1 AND 2 AND y.b = x.b;
-- 5, 6            expected 1, 2, 3, 4, 5, 6

-- A branch reads t2, whose cursor is never opened:
SELECT DISTINCT y.a FROM t1 AS x JOIN t1 AS y ON y.b >= x.b AND y.b <= x.b
  LEFT JOIN t2 ON t2.c = y.a OR y.a = 5
 WHERE y.b BETWEEN 1 AND 2 AND y.b = x.b;
-- segmentation fault (NULL VdbeCursor in OP_Column)       expected 1, 2, 3, 4, 5, 6
