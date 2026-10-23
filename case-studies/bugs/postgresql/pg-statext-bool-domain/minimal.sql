-- ERROR:  expression not found in statistics object (XX000, an elog)
CREATE DOMAIN db AS bool;
CREATE TABLE t(a int8, b db);
INSERT INTO t VALUES (1, true);
CREATE STATISTICS t_s (mcv) ON a, b FROM t;
ANALYZE t;
SELECT * FROM t WHERE a = 1 AND b;          -- ERROR
SELECT * FROM t WHERE a = 1 AND b = true;   -- ERROR
SELECT * FROM t WHERE a = 1 AND NOT b;      -- ERROR
