CREATE TABLE t1(c0 INTEGER);
SET disabled_optimizers='unused_columns';
SELECT 0 FROM t1 HAVING 0 IN (SELECT 0 FROM t1);
SELECT 42 AS still_alive;
