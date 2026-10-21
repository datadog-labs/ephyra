-- anti-strict: a pair sqleq-fuzz finds no counterexample for; PostgreSQL shows they differ on the rows below.
-- minimized from s14_q0_anti-strict-d_2
CREATE TABLE t0 (c0 numeric NOT NULL, PRIMARY KEY (c0), UNIQUE (c0));
SELECT NULL::float8 FROM t0 AS a0_0 WHERE (a0_0.c0 >= '-820'::numeric);
SELECT NULL::float8 FROM t0 AS a0_0 WHERE (a0_0.c0 > '-820'::numeric);

-- witness rows (load into the schema above):
INSERT INTO t0 (c0) VALUES ('-820'::numeric);

-- on the witness rows, query 1 returns 1 rows, query 2 returns 0 rows:
-- 1: [[Value(Null)]]
-- 2: []
