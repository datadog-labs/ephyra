\pset pager off
SET client_min_messages = warning;
CREATE TABLE t (id int, a int, b bool, c int);
INSERT INTO t VALUES (1, 1, true, 1), (2, 2, false, NULL), (3, NULL, NULL, 3), (4, 1, false, 2);
CREATE TABLE u (x bool, y bool);
INSERT INTO u VALUES (false, true), (true, false);

CREATE FUNCTION chk(name text, expr text) RETURNS TABLE (case_name text, verdict text, pretty_expr text, detail text)
LANGUAGE plpgsql AS $$
DECLARE
  orig text; pdef text; rerun text; npdef text; nprerun text;
BEGIN
  case_name := name;
  BEGIN
    EXECUTE format('CREATE VIEW %I AS SELECT id, %s AS r FROM t', name, expr);
  EXCEPTION WHEN others THEN
    verdict := 'NO VIEW'; detail := SQLERRM; RETURN NEXT; RETURN;
  END;
  EXECUTE format('SELECT string_agg(coalesce(r::text, ''null''), '','' ORDER BY id) FROM %I', name) INTO orig;
  pdef := pg_get_viewdef(name::regclass, true);
  npdef := pg_get_viewdef(name::regclass, false);
  pretty_expr := regexp_replace(regexp_replace(pdef, '\s+', ' ', 'g'), '^ ?SELECT id, (.*) AS r FROM t;$', '\1');
  BEGIN
    EXECUTE format('SELECT string_agg(coalesce(r::text, ''null''), '','' ORDER BY id) FROM (%s) s', rtrim(pdef, ';')) INTO rerun;
  EXCEPTION WHEN others THEN
    verdict := 'FAILS'; detail := 'orig=' || orig || ' | rerun error: ' || SQLERRM;
  END;
  IF verdict IS NULL THEN
    IF rerun IS NOT DISTINCT FROM orig THEN verdict := 'fine'; detail := 'orig=' || orig;
    ELSE verdict := 'CHANGED'; detail := 'orig=' || orig || ' | rerun=' || rerun; END IF;
  END IF;
  -- non-pretty control
  BEGIN
    EXECUTE format('SELECT string_agg(coalesce(r::text, ''null''), '','' ORDER BY id) FROM (%s) s', rtrim(npdef, ';')) INTO nprerun;
    IF nprerun IS DISTINCT FROM orig THEN detail := detail || ' | NONPRETTY DIFFERS: ' || coalesce(nprerun,'null'); END IF;
  EXCEPTION WHEN others THEN detail := detail || ' | NONPRETTY ERROR: ' || SQLERRM;
  END;
  RETURN NEXT;
END $$;

\x on
SELECT * FROM (VALUES
 ('in_eq',        '(a = 1) IN (SELECT b FROM u WHERE b IS NOT NULL LIMIT 0) OR (a = 1) IN (SELECT x FROM u)'),
 ('in_eq_min',    '(a = 1) IN (SELECT x FROM u)'),
 ('in_lt_bool',   '(b < b) IN (SELECT false)'),
 ('in_le',        '(a <= c) IN (SELECT x FROM u)'),
 ('in_ne',        '(a <> c) IN (SELECT x FROM u)'),
 ('in_gt_bool',   '(b > false) IN (SELECT false)'),
 ('in_isnull',    '(a IS NULL) IN (SELECT x FROM u)'),
 ('in_isnotnull', '(a IS NOT NULL) IN (SELECT x FROM u)'),
 ('in_istrue',    '(b IS TRUE) IN (SELECT x FROM u)'),
 ('in_isunknown', '(b IS UNKNOWN) IN (SELECT x FROM u)'),
 ('in_distinct',  '(a IS DISTINCT FROM c) IN (SELECT x FROM u)'),
 ('in_notdist',   '(a IS NOT DISTINCT FROM c) IN (SELECT x FROM u)'),
 ('in_distinct_b','(b IS DISTINCT FROM true) IN (SELECT false)'),
 ('in_not',       '(NOT b) IN (SELECT false)'),
 ('in_and',       '(b AND b) IN (SELECT false)'),
 ('in_or',        '(b OR true) IN (SELECT false)'),
 ('in_between',   '(a BETWEEN 1 AND 2) IN (SELECT x FROM u)'),
 ('in_like',      '(a::text LIKE ''1%'') IN (SELECT x FROM u)'),
 ('in_inlist',    '(a IN (1, 2)) IN (SELECT x FROM u)'),
 ('in_isjson',    '(a::text IS JSON) IN (SELECT x FROM u)'),
 ('in_plus',      '(a + 1) IN (SELECT c FROM t)'),
 ('in_sublink',   '(a IN (SELECT c FROM t)) IN (SELECT x FROM u)'),
 ('notin_lt',     '(b < b) NOT IN (SELECT false)'),
 ('notin_eq',     '(a = 1) NOT IN (SELECT x FROM u)'),
 ('notin_or',     '(b OR true) NOT IN (SELECT false)'),
 ('any_lt_eq',    '(a = 1) < ANY (SELECT x FROM u)'),
 ('any_ne_or',    '(b OR true) <> ANY (SELECT true)'),
 ('any_ne_not',   '(NOT b) <> ANY (SELECT true)'),
 ('any_ne_isnull','(a IS NULL) <> ANY (SELECT true)'),
 ('all_ne_lt',    '(b < b) <> ALL (SELECT true)'),
 ('all_lt_and',   '(b AND true) < ALL (SELECT true)'),
 ('all_ne_or',    '(b OR true) <> ALL (SELECT true)'),
 ('row_in',       '((a = 1), b) IN (SELECT x, y FROM u)'),
 ('row_in_or',    '((b OR true), b) IN (SELECT x, y FROM u)'),
 ('row_notin',    '((a = 1), (b < b)) NOT IN (SELECT x, y FROM u)'),
 ('row_lt_any',   '((a = 1), b) < ANY (SELECT x, y FROM u)'),
 ('row_cmp',      '((a = 1), (b OR true)) < (SELECT x, y FROM u LIMIT 1)'),
 ('in_not2',      '(NOT b) IN (SELECT x FROM u)'),
 ('any_lt_not',   '(NOT b) < ANY (SELECT false)'),
 ('in_isnull2',   '(a IS NULL) IN (SELECT true)'),
 ('in_istrue2',   '(b IS TRUE) IN (SELECT true)'),
 ('in_inlist2',   '(a IN (1, 2)) IN (SELECT true)'),
 ('in_like2',     '(a::text LIKE ''1%'') IN (SELECT true)'),
 ('in_isjson2',   '(a::text IS JSON) IN (SELECT false)'),
 ('in_collate',   '(a::text < ''2'' COLLATE "C") IN (SELECT true)'),
 ('expr_sublink', '(a = 1) = (SELECT x FROM u LIMIT 1)')
) v(n, e), LATERAL chk(n, e);
\x off
