\set VERBOSITY verbose
-- works: only a FOR ORDINALITY column
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING (SELECT 1) AS v
  COLUMNS (o FOR ORDINALITY)) jt;
-- fails: any PATH column
SELECT * FROM JSON_TABLE('[1,2]'::jsonb, '$[*]' PASSING (SELECT 1) AS v
  COLUMNS (a int PATH '$')) jt;
-- works: the same subquery in JSON_QUERY's PASSING
SELECT JSON_QUERY('[1,2]'::jsonb, '$[*] ? (@ == $v)' PASSING (SELECT 1) AS v);
