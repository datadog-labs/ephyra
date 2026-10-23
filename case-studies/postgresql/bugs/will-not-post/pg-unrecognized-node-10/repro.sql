SET debug_parallel_query = on; SET parallel_setup_cost = 0; SET parallel_tuple_cost = 0; SET min_parallel_table_scan_size = 0;
CREATE TABLE t1(c1 numeric, c2 int4);
INSERT INTO t1 DEFAULT VALUES;
SELECT pg_restore_relation_stats('schemaname', current_schema()::text, 'relname', 't1', 'relpages', 1000::integer, 'reltuples', '1000000'::real, 'relallvisible', 0::integer);
CREATE TABLE t2(c0 text, c2 int4);
CREATE TABLE t3(c0 text, c2 int8, c3 int4);
SELECT count(*) FROM t3 AS a0_0 RIGHT JOIN t2 AS a0_1 ON ((a0_1.c2 = a0_0.c3) AND (('5874897-12-31'::date - '2000-12-16'::date) <> '3'::int4)) JOIN t2 AS a0_2 ON (a0_2.c0 = a0_0.c0) WHERE (NOT EXISTS (SELECT CAST(a1_0.c1 AS float8) FROM t1 AS a1_0 JOIN t2 AS a1_1 ON (a1_1.c2 = a1_0.c2) WHERE ('0'::int8 IN (SELECT (a2_0.c2 + (a2_0.c2 / '2'::int8)) FROM t3 AS a2_0 JOIN t1 AS a2_1 ON (a2_1.c2 = a2_0.c3)))));
