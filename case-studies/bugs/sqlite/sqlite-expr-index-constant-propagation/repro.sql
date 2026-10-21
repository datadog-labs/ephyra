-- An index on an expression combined with WHERE-clause constant propagation
-- gives wrong results. A "col = constant" term makes constant propagation tag
-- col inside the indexed expression as EP_FixedCol, so the term that holds the
-- expression no longer depends on that table and is coded in an outer loop.
-- The code generator still swaps in the index column for the expression, and
-- reads it from an index cursor that the outer loop has not positioned yet.
-- Tested on trunk dc388e83b05d (2026-10-10), 3.53.4, and 3.41.0 to 3.49.0.

-- Query 1: the planner picks the expression index on its own (wrong since 3.41.0).
CREATE TABLE t1(c TEXT);
INSERT INTO t1 VALUES('');
CREATE INDEX t1c ON t1(CAST(c AS INTEGER));
SELECT count(*) FROM (SELECT 1) CROSS JOIN t1
 WHERE CAST(c AS INTEGER) IN (SELECT 0) AND c = '';
-- expected: 1   (CAST('' AS INTEGER) is 0, so both terms are true for the one row)
-- actual:   0

-- Query 2: INTEGER PRIMARY KEY, covering scan of the expression index (wrong since 3.40.0).
CREATE TABLE t2(a INTEGER PRIMARY KEY);
INSERT INTO t2 VALUES(-1),(0);
CREATE INDEX t2a ON t2(CAST(a AS INTEGER));
SELECT * FROM t2 AS x CROSS JOIN t2 AS y INDEXED BY t2a
 WHERE x.a <> CAST(y.a AS INTEGER) AND y.a = 0;
-- expected: -1|0
-- actual:   (no rows)

-- Query 3: CAST is not needed; any indexed expression over the propagated column does it.
CREATE TABLE t3(a INT);
INSERT INTO t3 VALUES(0);
CREATE INDEX t3a ON t3(a+1);
SELECT count(*) FROM (SELECT 1) CROSS JOIN t3
 WHERE a+1 IN (SELECT 1) AND a = 0;
-- expected: 1
-- actual:   0

-- Each query gives the right answer with ".testctrl optimizations -PropagateConst",
-- with "-IndexedExpr", with NOT INDEXED on the indexed table, or with fix.diff.
