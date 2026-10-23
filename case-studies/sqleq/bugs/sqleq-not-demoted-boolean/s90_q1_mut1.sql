-- ephyra-sqleq seed 90 query 1: mutant 1
CREATE TABLE t0 (c0 timestamp, c1 timestamp, c2 float4, c3 int2);
CREATE TABLE t1 (c0 int8, c1 int8, c2 float8, c3 bool, c5 int4);
CREATE TABLE t2 (c0 bytea NOT NULL);
CREATE TABLE t3 (c0 int8, c1 int4 NOT NULL, c2 bytea NOT NULL, PRIMARY KEY (c2));
SELECT * FROM t2 AS a0_0 JOIN t3 AS a0_1 ON (a0_1.c2 = a0_0.c0) JOIN t0 AS a0_2 ON ((ARRAY[a0_1.c0, CAST(a0_2.c3 AS int8), CAST(a0_2.c3 AS int8), CAST(a0_1.c1 AS int8)]::int8[] <@ array_append(ARRAY[NULL::int8, CAST(a0_2.c3 AS int8), a0_1.c0]::int8[], a0_1.c0)) IS FALSE);
SELECT * FROM t2 AS a0_0 JOIN t3 AS a0_1 ON (a0_1.c2 <> a0_0.c0) JOIN t0 AS a0_2 ON ((NOT (ARRAY[a0_1.c0, CAST(a0_2.c3 AS int8), CAST(a0_2.c3 AS int8), CAST(a0_1.c1 AS int8)]::int8[] <@ array_append(ARRAY[NULL::int8, CAST(a0_2.c3 AS int8), a0_1.c0]::int8[], a0_1.c0))) IS TRUE);
