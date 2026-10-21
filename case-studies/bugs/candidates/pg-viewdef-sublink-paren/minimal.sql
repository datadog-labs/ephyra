-- pg_get_viewdef(.., true) drops the parentheses around the left operand of
-- IN (subquery) / op ANY|ALL (subquery) when it is an operator expression,
-- and IN binds tighter than comparison operators, so the text means
-- something else.
CREATE TABLE t (a int, b bool);
INSERT INTO t VALUES (1, true);
CREATE VIEW v AS SELECT (a = 1) IN (SELECT b FROM t) AS r FROM t;
SELECT * FROM v;                              -- r = t
SELECT pg_get_viewdef('v'::regclass, true);   --  SELECT (a = 1 IN ( SELECT t_1.b FROM t t_1)) AS r FROM t;
SELECT pg_get_viewdef('v'::regclass, false);  -- keeps (a = 1)
-- Re-running the pretty text fails: a = (1 IN (SELECT b ..)) compares an
-- int with a boolean.
\set VERBOSITY default
SELECT (a = 1 IN ( SELECT t_1.b FROM t t_1)) AS r FROM t;
-- With operands of one type the text runs and gives other rows.
CREATE VIEW w AS SELECT (b < b) IN (SELECT false) AS r FROM t;
SELECT * FROM w;                              -- r = t: (true < true) IN (false)
SELECT pg_get_viewdef('w'::regclass, true);   --  SELECT (b < b IN ( SELECT false)) AS r FROM t;
SELECT (b < b IN ( SELECT false)) AS r FROM t;  -- r = f: true < (true IN (false))
