-- GROUP BY + ORDER BY over a subquery whose ORDER BY runs the other way:
-- the outer sort is skipped and rows come back in the subquery's order.
CREATE TABLE t(a);
INSERT INTO t VALUES(1),(2);

SELECT a FROM (SELECT a FROM t ORDER BY a LIMIT 2) GROUP BY a ORDER BY a DESC;
-- trunk c1ad888eec41: 1, 2      expected: 2, 1

.testctrl optimizations -OrderBySubq
SELECT a FROM (SELECT a FROM t ORDER BY a LIMIT 2) GROUP BY a ORDER BY a DESC;
-- 2, 1
