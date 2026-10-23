-- anti-strict: a pair sqleq-fuzz finds no counterexample for; PostgreSQL shows they differ on the rows below.
-- minimized from s12_q1_anti-strict-d_1
CREATE TABLE t0 (c1 date NOT NULL, c2 int8);
SELECT a0_0.c1 FROM t0 AS a0_0 WHERE (a0_0.c1 > '2000-01-01'::date);
SELECT a0_0.c1 FROM t0 AS a0_0 WHERE (a0_0.c1 >= '2000-01-01'::date);

-- witness rows (load into the schema above):
INSERT INTO t0 (c1, c2) VALUES ('2000-01-01'::date, '395'::int8);

-- on the witness rows, query 1 returns 0 rows, query 2 returns 1 rows:
-- 1: []
-- 2: [[Value(Date(0))]]
