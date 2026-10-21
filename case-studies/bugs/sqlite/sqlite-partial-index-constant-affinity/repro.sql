-- A partial index whose WHERE clause is "col = <constant>" makes the code
-- generator read col as <constant> with col's affinity applied, instead of the
-- stored value (3.44.0: "In partial index scans, if the WHERE clause implies a
-- constant value for a table column, replace occurrences of that table column
-- with the constant"). That is only right when col = <constant> forces the
-- stored value to be exactly that. It does not in two cases.
-- Tested on trunk dc388e83b05d (2026-10-10), 3.54.0, 3.53.4, 3.44.0 to 3.49.0.

-- Query 1 and 2: a TEXT column compared with a constant that has numeric
-- affinity. The comparison applies NUMERIC affinity to the column, so '5.0'
-- equals CAST(5 AS INT), but the column then reads back as '5'.
CREATE TABLE t1(a, b TEXT);
INSERT INTO t1 VALUES(1, '5.0');
CREATE INDEX t1a ON t1(a) WHERE b = CAST(5 AS INT);
SELECT b FROM t1 WHERE b = CAST(5 AS INT);
-- expected: 5.0
-- actual:   5
SELECT count(*) FROM t1 WHERE b = CAST(5 AS INT) AND b LIKE '%.0';
-- expected: 1
-- actual:   0

-- Query 3 and 4: an INT column holding the INTEGER -9223372036854775808,
-- compared with the REAL -9223372036854775808.0 (exactly -2^63, so the two are
-- equal). INT affinity does not turn that REAL into an INTEGER, so the column
-- reads back as a REAL.
CREATE TABLE t2(a, b INT);
INSERT INTO t2 VALUES(1, -9223372036854775808);
CREATE INDEX t2a ON t2(a) WHERE b = -9223372036854775808.0;
SELECT b, typeof(b) FROM t2 WHERE b = -9223372036854775808.0;
-- expected: -9223372036854775808|integer
-- actual:   -9.2233720368547758e+18|real

-- The other way round: a REAL -2^63 stored in an INT column (INT affinity
-- leaves it REAL) reads back as an INTEGER when the index says
-- b = -9223372036854775808.
CREATE TABLE t3(a, b INT);
INSERT INTO t3 VALUES(1, -9223372036854775808.0);
CREATE INDEX t3a ON t3(a) WHERE b = -9223372036854775808;
SELECT b, typeof(b) FROM t3 WHERE b = -9223372036854775808;
-- expected: -9.2233720368547758e+18|real
-- actual:   -9223372036854775808|integer

-- The fuzzer's case: a STRICT table, where an INT column can only hold integers.
CREATE TABLE t4(a ANY, b INT) STRICT;
INSERT INTO t4 VALUES(1, -9223372036854775808);
CREATE INDEX t4a ON t4(a) WHERE b = -9.223372036854776e18;
SELECT b, typeof(b) FROM t4 WHERE b = -9.223372036854776e18;
-- expected: -9223372036854775808|integer
-- actual:   -9.2233720368547758e+18|real

-- Every query is right with NOT INDEXED or with fix.diff. No
-- ".testctrl optimizations" bit turns the substitution off.
