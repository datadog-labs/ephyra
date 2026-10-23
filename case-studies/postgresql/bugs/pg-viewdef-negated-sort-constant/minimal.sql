-- pg_get_viewdef prints an ORDER BY / GROUP BY term that negates an integer
-- literal as (- 3); reparsed, - 3 folds into the constant -3, which ORDER
-- BY and GROUP BY read as a column position.
CREATE TABLE t (a int);
INSERT INTO t VALUES (1), (2);
CREATE VIEW o AS SELECT a FROM t ORDER BY (- '3'::int4);
CREATE VIEW g AS SELECT count(*) AS n FROM t GROUP BY (- '3'::int4);
SELECT * FROM o;
SELECT * FROM g;                               -- n = 2
SELECT pg_get_viewdef('o'::regclass, false);
SELECT pg_get_viewdef('g'::regclass, false);
\set VERBOSITY default
-- Re-running either text fails:
SELECT t.a FROM t ORDER BY (- 3);
SELECT count(*) AS n FROM t GROUP BY (- 3);
