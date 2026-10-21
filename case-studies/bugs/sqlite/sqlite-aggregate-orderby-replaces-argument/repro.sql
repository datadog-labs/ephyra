-- An aggregate with an ORDER BY of its own (3.44.0+) aggregates its ORDER BY
-- term instead of its argument when the two are the same expression over
-- different tables and the ORDER BY term's table is the first one in FROM.
-- Run as: sqlite3 :memory: < repro.sql
CREATE TABLE t1(a);
CREATE TABLE t2(b);
INSERT INTO t1 VALUES(1);
INSERT INTO t2 VALUES(2);
SELECT sum(b ORDER BY a) FROM t1, t2;    -- 1, expected 2
SELECT sum(b) FROM t1, t2;               -- 2
SELECT sum(b ORDER BY a) FROM t2, t1;    -- 2 (FROM order swapped)
SELECT sum(b ORDER BY +a) FROM t1, t2;   -- 2 (unary plus on the ORDER BY term)

-- The fuzzer's query: the NULL-extended a1.c1 is replaced by a0.c1.
CREATE TABLE t3(c0, c1);
INSERT INTO t3 VALUES(1, 2);
SELECT count(a1.c1 ORDER BY a0.c1) FROM t3 AS a0 LEFT JOIN t3 AS a1 ON NULL;   -- 1, expected 0
SELECT count(a1.c1) FROM t3 AS a0 LEFT JOIN t3 AS a1 ON NULL;                  -- 0
SELECT count(a1.c1 ORDER BY a0.c1) FROM t3 AS a1 RIGHT JOIN t3 AS a0 ON NULL;  -- 0

-- Values come from the other row: expected y,x
CREATE TABLE t4(k, v);
INSERT INTO t4 VALUES(1, 'x'), (2, 'y');
SELECT group_concat(q.v ORDER BY p.v) FROM t4 AS p, t4 AS q WHERE p.k<>q.k;    -- x,y, expected y,x
