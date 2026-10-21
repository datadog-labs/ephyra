\pset pager off
\pset format unaligned
\pset fieldsep ' | '
CREATE TABLE t (a int, b int, c int);
INSERT INTO t VALUES (1, 10, 3), (2, 20, 2), (3, 30, 1), (1, 40, 0);
-- recheck(view, pretty): run the deparsed text again as a new view, report its deparse and whether the rows match
CREATE FUNCTION recheck(v text, pretty bool) RETURNS text LANGUAGE plpgsql AS $f$
DECLARE d text; d2 text; r1 text; r2 text; nv text := v || CASE WHEN pretty THEN '_rp' ELSE '_re' END;
BEGIN
  d := pg_get_viewdef(v::regclass, pretty);
  BEGIN
    EXECUTE format('CREATE VIEW %I AS %s', nv, rtrim(d, E';\n '));
  EXCEPTION WHEN others THEN RETURN 'REPARSE ERROR: ' || sqlerrm;
  END;
  d2 := pg_get_viewdef(nv::regclass, false);
  EXECUTE format('SELECT string_agg(x::text, %L) FROM %I x', ' ', v) INTO r1;
  EXECUTE format('SELECT string_agg(x::text, %L) FROM %I x', ' ', nv) INTO r2;
  RETURN CASE WHEN r1 IS NOT DISTINCT FROM r2 THEN 'same rows' ELSE 'ROWS DIFFER: ' || r1 || '  vs  ' || r2 END
         || ' ; redeparse: ' || regexp_replace(d2, '\s+', ' ', 'g');
END $f$;
CREATE TABLE cases (v text, ddl text);
INSERT INTO cases VALUES
 -- literal types, ORDER BY
 ('o_int4',   'SELECT a FROM t ORDER BY - ''3''::int4'),
 ('o_int4_0', 'SELECT a FROM t ORDER BY - ''0''::int4'),
 ('o_int4_neg','SELECT a FROM t ORDER BY - ''-1''::int4'),
 ('o_int4_neg3','SELECT a, b, c FROM t ORDER BY - ''-3''::int4'),
 ('o_int4_dbl','SELECT a, b, c FROM t ORDER BY - (- ''3''::int4)'),
 ('o_int4_dbl1','SELECT a, b, c FROM t ORDER BY - (- ''1''::int4)'),
 ('o_int2',   'SELECT a FROM t ORDER BY - ''3''::int2'),
 ('o_int8',   'SELECT a FROM t ORDER BY - ''3''::int8'),
 ('o_num_int','SELECT a FROM t ORDER BY - ''3''::numeric'),
 ('o_num_dec','SELECT a FROM t ORDER BY - ''1.5''::numeric'),
 ('o_num_dec_dbl','SELECT a FROM t ORDER BY - (- ''1.5''::numeric)'),
 ('o_float8', 'SELECT a FROM t ORDER BY - 1.5::float8'),
 ('o_plus',   'SELECT a FROM t ORDER BY + ''3''::int4'),
 ('o_minus_plus','SELECT a, b, c FROM t ORDER BY - (+ ''3''::int4)'),
 ('o_binop',  'SELECT a FROM t ORDER BY (- ''3''::int4) + 0'),
 -- clauses (int4)
 ('g_int4',   'SELECT count(*) AS n FROM t GROUP BY - ''3''::int4'),
 ('g_dbl',    'SELECT a, count(*) AS n FROM t GROUP BY a, - (- ''1''::int4)'),
 ('do_int4',  'SELECT DISTINCT ON (- ''3''::int4) a FROM t'),
 ('do_dbl',   'SELECT DISTINCT ON (- (- ''1''::int4)) a, b FROM t ORDER BY - (- ''1''::int4), b DESC'),
 ('w_order',  'SELECT a, row_number() OVER (ORDER BY - ''3''::int4) AS rn FROM t'),
 ('w_part',   'SELECT a, count(*) OVER (PARTITION BY - ''3''::int4) AS n FROM t'),
 ('w_named',  'SELECT a, count(*) OVER w AS n FROM t WINDOW w AS (PARTITION BY - ''3''::int4 ORDER BY - ''3''::int4)'),
 ('gs_sets',  'SELECT count(*) AS n FROM t GROUP BY GROUPING SETS ((- ''3''::int4), ())'),
 ('gs_rollup','SELECT count(*) AS n FROM t GROUP BY ROLLUP (- ''3''::int4)'),
 ('gs_cube',  'SELECT count(*) AS n FROM t GROUP BY CUBE (- ''3''::int4)'),
 ('agg_order','SELECT array_agg(a ORDER BY - ''3''::int4) AS x FROM t'),
 ('setop_order','SELECT a FROM t UNION ALL SELECT b FROM t ORDER BY 1'),
 ('sub_order','SELECT * FROM (SELECT a FROM t ORDER BY - ''3''::int4 LIMIT 2) s'),
 ('distinct', 'SELECT DISTINCT - ''3''::int4 AS x FROM t');
DO $d$ DECLARE r record; BEGIN
  FOR r IN SELECT * FROM cases LOOP
    BEGIN EXECUTE format('CREATE VIEW %I AS %s', r.v, r.ddl);
    EXCEPTION WHEN others THEN RAISE NOTICE 'create % failed: %', r.v, sqlerrm; END;
  END LOOP; END $d$;
SELECT c.v, regexp_replace(pg_get_viewdef(c.v::regclass, false), '\s+', ' ', 'g') AS raw,
       regexp_replace(pg_get_viewdef(c.v::regclass, true), '\s+', ' ', 'g') AS pretty
FROM cases c ORDER BY c.v;
SELECT c.v, recheck(c.v, false) AS raw_reparse, recheck(c.v, true) AS pretty_reparse FROM cases c ORDER BY c.v;
-- other deparse users
CREATE TABLE log (x int);
CREATE RULE r_ins AS ON INSERT TO log DO ALSO INSERT INTO log SELECT a FROM t GROUP BY a, - (- '2'::int4) LIMIT 0;
SELECT pg_get_ruledef(oid) FROM pg_rewrite WHERE rulename = 'r_ins';
SELECT pg_get_ruledef(oid, true) FROM pg_rewrite WHERE rulename = 'r_ins';
SELECT replace(pg_get_ruledef(oid), 'r_ins', 'r_ins2') FROM pg_rewrite WHERE rulename = 'r_ins' \gexec
CREATE MATERIALIZED VIEW mv AS SELECT a FROM t ORDER BY - '3'::int4;
SELECT pg_get_viewdef('mv'::regclass);
CREATE FUNCTION sqlf() RETURNS SETOF int LANGUAGE sql BEGIN ATOMIC SELECT a FROM t ORDER BY - '3'::int4; END;
SELECT pg_get_functiondef('sqlf'::regproc);
SELECT * FROM sqlf();
SELECT replace(pg_get_functiondef('sqlf'::regproc), 'sqlf', 'sqlf2') \gexec
SELECT format('CREATE MATERIALIZED VIEW mv2 AS %s', rtrim(pg_get_viewdef('mv'::regclass), E';\n ')) \gexec
