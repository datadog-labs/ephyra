CREATE TABLE t0(c0, c1);
CREATE TABLE t1(c0, c1);
INSERT INTO t0 VALUES (1, 2), (4, 5);
INSERT INTO t1 VALUES (1, 3), (6, 7);
-- Each is "ambiguous column name: c0"; each should return its rows.
SELECT * FROM t0 FULL JOIN t1 USING (c0) CROSS JOIN t0 AS t2;
SELECT * FROM t0 RIGHT JOIN t1 USING (c0) CROSS JOIN t0 AS t2;
SELECT * FROM t0 JOIN t1 USING (c0) RIGHT JOIN t0 AS t2 ON 1;
