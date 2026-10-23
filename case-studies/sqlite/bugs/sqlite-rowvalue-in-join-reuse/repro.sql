-- A row-value IN whose first field is joined by equality to another table's
-- indexed column. The inner loop takes the IN term through the equivalence,
-- removeUnindexableInClauseTerms() drops the unused field but keeps
-- EP_Subrtn, and sqlite3CodeRhsOfIN() reuses the full two-column list
-- (OP_OpenDup): the inner loop reads column 0 of every row, duplicates
-- included, so each joined row comes out once per matching list element.
CREATE TABLE t1(a PRIMARY KEY, b);
INSERT INTO t1 VALUES(1, 1);
ANALYZE;
PRAGMA integrity_check;
SELECT count(*) FROM t1 x, t1 y WHERE x.a = y.a AND (x.a, x.b) IN ((1, 1), (1, 2));
-- 2               expected 1
SELECT x.a, x.b FROM t1 x, t1 y WHERE x.a = y.a AND (x.a, x.b) IN ((1, 1), (1, 2), (1, 3));
-- 1|1 1|1 1|1     expected 1|1
SELECT sum(((x.a, x.b) IN ((1, 1), (1, 2))) IS TRUE) FROM t1 x, t1 y WHERE x.a = y.a;
-- 1 (right)

-- The form the fuzzer found: WITHOUT ROWID, column repeated in the vector.
CREATE TABLE t3(c0, PRIMARY KEY(c0)) WITHOUT ROWID;
INSERT INTO t3 VALUES('k');
ANALYZE;
SELECT count(*) FROM t3 AS a0 JOIN t3 AS a1 USING (c0) WHERE (a0.c0, a0.c0) IN (('k', NULL), ('k', 'k'));
-- 2               expected 1
