CREATE TABLE t(a REAL, b AS (a) VIRTUAL);
INSERT INTO t(a) VALUES(1.0);
CREATE INDEX i ON t(b);
SELECT b, typeof(b) FROM t NOT INDEXED;   -- 1.0|real
SELECT b, typeof(b) FROM t INDEXED BY i;  -- 1|integer, expected 1.0|real
SELECT b, typeof(b) FROM t WHERE b = 1;   -- 1|integer, expected 1.0|real
