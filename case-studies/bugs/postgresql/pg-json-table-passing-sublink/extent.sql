\set VERBOSITY verbose
\echo === A1 uncorrelated scalar subquery, PATH column
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*] ? (@ == $v)' PASSING (SELECT 1) AS v COLUMNS (a int PATH '$')) jt;
\echo === A2 correlated scalar subquery
CREATE TABLE t (id int, j jsonb, k int);
INSERT INTO t VALUES (1, '[1,2,3]', 2), (2, '[4,5]', 5);
SELECT t.id, jt.* FROM t, JSON_TABLE(t.j, '$[*] ? (@ == $v)' PASSING (SELECT t.k) AS v COLUMNS (a int PATH '$')) jt;
SELECT t.id, jt.* FROM t, JSON_TABLE(t.j, '$[*] ? (@ == $v)' PASSING (SELECT max(k) FROM t t2 WHERE t2.id = t.id) AS v COLUMNS (o FOR ORDINALITY)) jt;
\echo === A3 EXISTS and ARRAY subqueries
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING EXISTS (SELECT 1) AS v COLUMNS (a int PATH '$')) jt;
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING ARRAY(SELECT 1) AS v COLUMNS (a int PATH '$')) jt;
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING 1 IN (SELECT 1) AS v COLUMNS (a int PATH '$')) jt;
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING EXISTS (SELECT 1) AS v COLUMNS (o FOR ORDINALITY)) jt;
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING ARRAY(SELECT 1) AS v COLUMNS (o FOR ORDINALITY)) jt;
\echo === A4 subquery nested inside an expression / cast / function
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING 1 + (SELECT 1) AS v COLUMNS (a int PATH '$')) jt;
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING coalesce((SELECT 1), 0) AS v COLUMNS (a int PATH '$')) jt;
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING (SELECT 1)::text AS v COLUMNS (a int PATH '$')) jt;
\echo === A5 column kinds: EXISTS, FORMAT JSON, jsonb type (formatted), default path
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (e bool EXISTS PATH '$')) jt;
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (f text FORMAT JSON PATH '$')) jt;
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (f jsonb PATH '$')) jt;
SELECT * FROM JSON_TABLE('[{"a":1}]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (a int)) jt;
\echo === A6 several PATH columns
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (a int PATH '$', b int PATH '$', c int PATH '$')) jt;
\echo === A7 several PASSING args, subquery not referenced by any path
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING 1 AS w, (SELECT 1) AS v COLUMNS (a int PATH '$')) jt;
\echo === A8 NESTED PATH: ordinality only (top and nested) / PATH col only in nested
SELECT * FROM JSON_TABLE('[[1,2]]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (o FOR ORDINALITY, NESTED PATH '$[*]' COLUMNS (o2 FOR ORDINALITY))) jt;
SELECT * FROM JSON_TABLE('[[1,2]]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (o FOR ORDINALITY, NESTED PATH '$[*] ? (@ == $v)' COLUMNS (o2 FOR ORDINALITY))) jt;
SELECT * FROM JSON_TABLE('[[1,2]]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (o FOR ORDINALITY, NESTED PATH '$[*]' COLUMNS (x int PATH '$'))) jt;
\echo === A9 view / prepared statement / SQL function
CREATE VIEW v_ok AS SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*] ? (@ == $v)' PASSING (SELECT 1) AS v COLUMNS (o FOR ORDINALITY)) jt;
SELECT * FROM v_ok;
SELECT pg_get_viewdef('v_ok');
CREATE VIEW v_bad AS SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (a int PATH '$')) jt;
PREPARE p AS SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (a int PATH '$')) jt;
PREPARE p2(int) AS SELECT * FROM JSON_TABLE('[1,2,3]'::jsonb, '$[*] ? (@ >= $v)' PASSING $1 AS v COLUMNS (a int PATH '$ ? (@ >= $v)', b int PATH '$')) jt;
EXECUTE p2(2);
PREPARE p3 AS SELECT * FROM JSON_TABLE('[1,2,3]'::jsonb, '$[*] ? (@ >= $v)' PASSING $1 AS v COLUMNS (a int PATH '$ ? (@ >= $v)', b int PATH '$')) jt;
EXECUTE p3(2);
SELECT parameter_types FROM pg_prepared_statements WHERE name = 'p3';
CREATE FUNCTION f_bad() RETURNS SETOF int LANGUAGE sql BEGIN ATOMIC SELECT a FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (a int PATH '$')) jt; END;
CREATE FUNCTION f_bad2() RETURNS SETOF int LANGUAGE sql AS $$ SELECT a FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING (SELECT 1) AS v COLUMNS (a int PATH '$')) jt $$;
SELECT * FROM f_bad2();
\echo === A10 subquery in the context item (not PASSING) and in a column DEFAULT
SELECT * FROM JSON_TABLE((SELECT '[1,2]'::jsonb), '$[*]' COLUMNS (a int PATH '$')) jt;
SELECT * FROM JSON_TABLE('[1,"x"]'::jsonb, '$[*]' COLUMNS (a int PATH '$' DEFAULT (SELECT 9) ON ERROR)) jt;
\echo === B volatile function in PASSING: how often is it evaluated, and do the row pattern and columns see the same value?
CREATE SEQUENCE s;
SELECT * FROM JSON_TABLE('[1,2,3]'::jsonb, '$[*]' PASSING nextval('s') AS v COLUMNS (o FOR ORDINALITY)) jt;
SELECT currval('s') AS evals_ordinality_only;
SELECT setval('s', 1, false);
SELECT * FROM JSON_TABLE('[1,2,3]'::jsonb, '$[*]' PASSING nextval('s') AS v COLUMNS (o FOR ORDINALITY, a int PATH '$v', b int PATH '$v')) jt;
SELECT currval('s') AS evals_two_path_cols;
SELECT setval('s', 1, false);
SELECT JSON_QUERY('[1,2,3]'::jsonb, '$[*] ? (@ >= $v)' PASSING nextval('s') AS v WITH WRAPPER);
SELECT currval('s') AS evals_json_query;
SELECT setval('s', 1, false);
-- row pattern filter and column filter use "the same" $v
SELECT * FROM JSON_TABLE('[1,2,3,4,5]'::jsonb, '$[*] ? (@ >= $v)' PASSING nextval('s') AS v COLUMNS (x int PATH '$', vv int PATH '$v', ok bool EXISTS PATH '$ ? (@ >= $v)')) jt;
SELECT currval('s') AS evals_filter;
\echo === B2 random(): each column sees a different value
SELECT count(DISTINCT (a, b)) FILTER (WHERE a <> b) AS rows_with_a_ne_b, count(*) FROM JSON_TABLE('[1,2,3,4,5,6,7,8,9,10]'::jsonb, '$[*]' PASSING random() AS r COLUMNS (a float8 PATH '$r', b float8 PATH '$r')) jt;
\echo === B3 EXPLAIN VERBOSE of a two-column JSON_TABLE with nextval
EXPLAIN (VERBOSE, COSTS OFF) SELECT * FROM JSON_TABLE('[1,2,3]'::jsonb, '$[*]' PASSING nextval('s') AS v COLUMNS (a int PATH '$v', b int PATH '$v')) jt;
CREATE VIEW v_vol AS SELECT * FROM JSON_TABLE('[1,2,3]'::jsonb, '$[*]' PASSING nextval('s') AS v COLUMNS (a int PATH '$v', b int PATH '$v')) jt;
SELECT pg_get_viewdef('v_vol');
