-- A compound SELECT (UNION, EXCEPT or INTERSECT) whose ORDER BY has a
-- COLLATE is rewritten as SELECT * FROM (<compound>) ORDER BY ...
-- (convertCompoundSelectToSubquery in select.c). The rewrite moves the
-- GROUP BY, HAVING and DISTINCT of the compound's last SELECT to the new
-- outer query instead of leaving them on that SELECT.
CREATE TABLE t1(a);
INSERT INTO t1 VALUES('x'), ('y'), ('y');
CREATE TABLE t2(a, b);
INSERT INTO t2 VALUES('x', 1), ('z', 2), ('z', 3);

SELECT a FROM t1 UNION SELECT a FROM t2 GROUP BY a HAVING count(*)>1
 ORDER BY a COLLATE NOCASE;
-- (no rows)       expected x, y, z

SELECT a FROM t1 EXCEPT SELECT a FROM t2 GROUP BY a HAVING count(*)>1
 ORDER BY a COLLATE NOCASE;
-- (no rows)       expected x, y

-- A GROUP BY term naming a column the compound's result does not have:
SELECT a FROM t1 UNION SELECT b FROM t2 GROUP BY b ORDER BY 1 COLLATE NOCASE;
-- no such column: b       expected 1, 2, 3, x, y

-- DISTINCT on the last SELECT applies to the whole compound:
SELECT 1 UNION SELECT 2 UNION ALL SELECT DISTINCT 1 ORDER BY 1 COLLATE NOCASE;
-- 1, 2            expected 1, 1, 2
