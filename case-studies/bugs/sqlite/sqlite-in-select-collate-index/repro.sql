-- "x IN (SELECT y COLLATE NOCASE ...)" uses NOCASE when evaluated, but the
-- planner still looks x up in a BINARY index on x, so the case-folded matches
-- are lost once that index exists.
-- Run as: sqlite3 :memory: < repro.sql
CREATE TABLE t1(a);
INSERT INTO t1 VALUES('a'), ('A');
CREATE TABLE t2(b);
INSERT INTO t2 VALUES('A');
SELECT group_concat(a) FROM t1 WHERE a IN (SELECT b COLLATE NOCASE FROM t2);        -- a,A
CREATE INDEX t1a ON t1(a);
SELECT group_concat(a) FROM t1 WHERE a IN (SELECT b COLLATE NOCASE FROM t2);        -- A, expected a,A
SELECT group_concat(a) FROM t1 NOT INDEXED WHERE a IN (SELECT b COLLATE NOCASE FROM t2);  -- a,A
SELECT group_concat(a) FROM t1 WHERE (a,1) IN (SELECT b COLLATE NOCASE, 1 FROM t2); -- a,A (vector form is right)
SELECT a, a IN (SELECT b COLLATE NOCASE FROM t2) FROM t1;                           -- a|1 and A|1

-- The fuzzer's form: the COLLATE is inside a function argument of the
-- subquery's result column.
SELECT group_concat(a) FROM t1 WHERE a IN (SELECT max(b COLLATE NOCASE) FROM t2);   -- A, expected a,A
