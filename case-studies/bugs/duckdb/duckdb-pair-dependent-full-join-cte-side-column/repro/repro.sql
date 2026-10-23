CREATE TABLE t(a INT, b DOUBLE);
INSERT INTO t VALUES (1, 2);
SET disabled_optimizers='projection_pullup';
SELECT y.a FROM t x CROSS JOIN t y FULL JOIN t z ON (SELECT x.b + y.b + z.b) > 0;
