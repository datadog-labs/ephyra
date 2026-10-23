-- In a GROUP BY query, `term IN (SELECT x ..)` in the HAVING clause (or the
-- items) applies the IN's comparison affinity to the GROUP BY term itself,
-- so the term reads converted everywhere in the query. Without GROUP BY, or
-- in WHERE, the values are not converted.
-- Expected: 5|text and 3.0|real on every line. SQLite 3.50.2 and 3.53.4
-- give 5|integer and 3|integer on the HAVING lines.
CREATE TABLE t0(c0);
INSERT INTO t0 VALUES ('5');
CREATE TABLE t1(c0 INTEGER);
CREATE TABLE t2(c0 REAL);
INSERT INTO t2 VALUES (3.0);
SELECT c0, typeof(c0) FROM t0 GROUP BY c0;
SELECT c0, typeof(c0) FROM t0 WHERE c0 NOT IN (SELECT c0 FROM t1);
SELECT c0, typeof(c0) FROM t0 GROUP BY c0 HAVING c0 NOT IN (SELECT c0 FROM t1);
SELECT c0, typeof(c0) FROM t2 GROUP BY c0;
SELECT c0, typeof(c0) FROM t2 GROUP BY c0 HAVING c0 NOT IN (SELECT c0 FROM t0);
-- An aggregate as the left operand is converted too (TEXT affinity here):
-- expected 1|integer, SQLite gives 1|text.
CREATE TABLE t3(c0 TEXT);
SELECT count(*), typeof(count(*)) FROM t0 GROUP BY NULL HAVING count(*) NOT IN (SELECT c0 FROM t3);
