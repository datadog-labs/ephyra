\set VERBOSITY terse
CREATE TABLE rt(c0 int4);
CREATE INDEX ON rt(c0);
CREATE TABLE nt(c0 int4);  -- no index
\echo A1 max, array_append (baseline)
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ANY (array_append(ARRAY[2], s.c0));
\echo A2 min
SELECT * FROM (SELECT min(c0) FROM rt) AS s(c0) WHERE 1 = ANY (array_append(ARRAY[2], s.c0));
\echo A3 no index
SELECT * FROM (SELECT max(c0) FROM nt) AS s(c0) WHERE 1 = ANY (array_append(ARRAY[2], s.c0));
\echo A4 ARRAY[s.c0, 2]
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ANY (ARRAY[s.c0, 2]);
\echo A5 IN list
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 IN (s.c0, 2);
\echo A6 s.c0 = ANY(array const)
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE s.c0 = ANY ('{1,2}'::int4[]);
\echo B1 array_prepend
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ANY (array_prepend(s.c0, ARRAY[2]));
\echo B2 array_cat
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ANY (array_cat(ARRAY[s.c0], ARRAY[2]));
\echo B3 array_remove
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ANY (array_remove(ARRAY[2, 3], s.c0));
\echo B4 || operator
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ANY (ARRAY[2] || s.c0);
\echo B5 CASE returning array
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ANY (CASE WHEN s.c0 > 0 THEN ARRAY[1] ELSE ARRAY[2] END);
\echo B6 string_to_array (other function)
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE '1' = ANY (string_to_array(s.c0::text, ','));
\echo B7 scalar function in non-array qual (abs)
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE abs(s.c0) = 1;
\echo B8 array_append of const only (no s.c0 in array)
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE s.c0 = ANY (array_append(ARRAY[2], 3));
\echo C1 ALL
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ALL (array_append(ARRAY[2], s.c0));
\echo C2 < ANY
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 < ANY (array_append(ARRAY[2], s.c0));
\echo C3 <> ALL (NOT IN-like)
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 <> ALL (array_append(ARRAY[2], s.c0));
\echo C4 array @> (no SAOP)
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE array_append(ARRAY[2], s.c0) @> ARRAY[1];
\echo C5 IN (subquery) instead of ANY(array)
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 IN (SELECT unnest(array_append(ARRAY[2], s.c0)));
\echo D1 LATERAL
SELECT * FROM (VALUES (1)) v(x), LATERAL (SELECT max(c0) FROM rt) AS s(c0) WHERE v.x = ANY (array_append(ARRAY[2], s.c0));
\echo D2 LATERAL referencing outer in subquery
SELECT * FROM (VALUES (1)) v(x), LATERAL (SELECT max(c0) FROM rt WHERE c0 > v.x) AS s(c0) WHERE 1 = ANY (array_append(ARRAY[2], s.c0));
\echo D3 CTE (inlined)
WITH s(c0) AS (SELECT max(c0) FROM rt) SELECT * FROM s WHERE 1 = ANY (array_append(ARRAY[2], s.c0));
\echo D4 CTE MATERIALIZED
WITH s(c0) AS MATERIALIZED (SELECT max(c0) FROM rt) SELECT * FROM s WHERE 1 = ANY (array_append(ARRAY[2], s.c0));
\echo D5 view
CREATE VIEW vmax(c0) AS SELECT max(c0) FROM rt;
SELECT * FROM vmax s WHERE 1 = ANY (array_append(ARRAY[2], s.c0));
\echo E1 no subquery: HAVING
SELECT max(c0) FROM rt HAVING 1 = ANY (array_append(ARRAY[2], max(c0)));
\echo E2 no subquery: target list
SELECT 1 = ANY (array_append(ARRAY[2], max(c0))) FROM rt;
\echo E3 no subquery: target list, min
SELECT 1 = ANY (array_append(ARRAY[2], min(c0))) FROM rt;
\echo E4 no subquery: target list, no index
SELECT 1 = ANY (array_append(ARRAY[2], max(c0))) FROM nt;
\echo E5 no subquery: target list with count (not min/max-only)
SELECT 1 = ANY (array_append(ARRAY[2], max(c0))), count(*) FROM rt;
\echo E6 target list, ARRAY[max(c0)]
SELECT 1 = ANY (ARRAY[max(c0), 2]) FROM rt;
\echo E7 target list, array_agg-free array var (int4[] column)
CREATE TABLE at(a int4[]); CREATE INDEX ON at(a);
SELECT 1 = ANY (max(a)) FROM at;
\echo E8 HAVING, GROUP BY (planagg not used)
SELECT max(c0) FROM rt GROUP BY () HAVING 1 = ANY (array_append(ARRAY[2], max(c0)));
\echo F1 with data and ANALYZE
INSERT INTO rt SELECT g FROM generate_series(1, 1000) g; ANALYZE rt;
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ANY (array_append(ARRAY[2], s.c0));
SELECT 1 = ANY (array_append(ARRAY[2], max(c0))) FROM rt;
\echo F2 with data and ANALYZE, no index
INSERT INTO nt SELECT g FROM generate_series(1, 1000) g; ANALYZE nt;
SELECT * FROM (SELECT max(c0) FROM nt) AS s(c0) WHERE 1000 = ANY (array_append(ARRAY[2], s.c0));
\echo F3 with data, matching row, index
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1000 = ANY (array_append(ARRAY[2], s.c0));
\echo G1 enable_indexscan/bitmap/indexonly off
SET enable_indexscan = off; SET enable_bitmapscan = off; SET enable_indexonlyscan = off;
SELECT 1 = ANY (array_append(ARRAY[2], max(c0))) FROM rt;
RESET ALL;
\echo G2 EXPLAIN
EXPLAIN (COSTS OFF) SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ANY (array_append(ARRAY[2], s.c0));
\echo G3 EXPLAIN of non-failing ARRAY[] form (shows pushdown into HAVING/One-Time Filter)
EXPLAIN (COSTS OFF) SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ANY (ARRAY[s.c0, 2]);
\echo G4 primary key instead of plain index
CREATE TABLE pk(c0 int4 PRIMARY KEY);
SELECT 1 = ANY (array_append(ARRAY[2], max(c0))) FROM pk;
\echo G5 text column
CREATE TABLE tt(c0 text); CREATE INDEX ON tt(c0);
SELECT 'a' = ANY (array_append(ARRAY['b'], max(c0))) FROM tt;
