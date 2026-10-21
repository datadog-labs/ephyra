-- PostgreSQL master 158b6d5109 / REL_19_STABLE d68601258e: ERROR XX000 unrecognized node type: <garbage>
-- (value varies: 7, 10, 11...; it is whatever reused the freed Path chunk).  REL_18_STABLE tip returns f.
SET parallel_setup_cost = 0;
SET parallel_tuple_cost = 0;
SET min_parallel_table_scan_size = 0;
CREATE TABLE t1(c2 int4);
INSERT INTO t1 DEFAULT VALUES;
SELECT pg_restore_relation_stats('schemaname', 'public', 'relname', 't1',
       'relpages', 1000::integer, 'reltuples', '1000000'::real);
CREATE TABLE t3(c0 text, c2 int8, c3 int4);
SELECT 0::int8 IN (SELECT t3.c2 + 1 FROM t3 JOIN t1 ON t1.c2 = t3.c3);
