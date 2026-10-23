-- anti-const: a pair sqleq-fuzz finds no counterexample for; PostgreSQL shows they differ on the rows below.
-- minimized from s3_q2_anti-const-d_2
CREATE TABLE t0 (c0 int4 NOT NULL, c1 int8 NOT NULL, PRIMARY KEY (c0, c1));
SELECT * FROM t0 AS a0_0 WHERE ('334'::int8 <= a0_0.c1);
SELECT * FROM t0 AS a0_0 WHERE ('335'::int8 <= a0_0.c1);

-- witness rows (load into the schema above):
INSERT INTO t0 (c0, c1) VALUES ('-232'::int4, '334'::int8);

-- on the witness rows, query 1 returns 1 rows, query 2 returns 0 rows:
-- 1: [[Value(Int(-232)), Value(Int(334))]]
-- 2: []
