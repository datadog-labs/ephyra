-- A UNION of four or more SELECTs is merged as two halves (the balanced
-- merge, SQLITE_BalancedMerge). The right half removes its duplicates under
-- its own collation instead of the whole compound's.
CREATE TABLE t0(c0);
CREATE TABLE t1(c0 COLLATE RTRIM);
INSERT INTO t1 VALUES (''), (' ');
.mode quote

-- t0.c0 (leftmost SELECT) is BINARY, so the compound is BINARY and
-- '' and ' ' are different rows.
SELECT c0 FROM t0 UNION SELECT 1 UNION SELECT 2 UNION SELECT c0 FROM t1;
-- 1, 2, ''            expected 1, 2, '', ' '

SELECT c0 FROM t0 UNION SELECT 1 UNION SELECT 2 UNION SELECT c0 FROM t1
 ORDER BY 1;
-- 1, 2, ''            expected 1, 2, '', ' '

-- No table needed; the UNIONs can be the left operand of another operator:
SELECT 1 COLLATE BINARY UNION SELECT 2 UNION SELECT '' UNION SELECT ' ' COLLATE RTRIM
 EXCEPT SELECT 9;
-- 1, 2, ''            expected 1, 2, '', ' '

-- The same dedup collation in the left operand of a merge (every release
-- tried): the compound is NOCASE (its first SELECT with a collation is the
-- last), 'a' and 'A' from different SELECTs are merged, from one are not.
CREATE TABLE t(c);
INSERT INTO t VALUES ('a'), ('A');
SELECT 'a' UNION SELECT 'A' UNION ALL SELECT 'x' COLLATE NOCASE ORDER BY 1;
-- 'A', 'x'
SELECT c || '' FROM t UNION SELECT 'b' UNION ALL SELECT 'x' COLLATE NOCASE ORDER BY 1;
-- 'a', 'A', 'b', 'x'  expected one of 'a'/'A', then 'b', 'x'
