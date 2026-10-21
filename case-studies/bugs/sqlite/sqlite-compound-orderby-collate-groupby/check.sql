-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-compound-orderby-collate-groupby
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t1(a);
INSERT INTO t1 VALUES('x'), ('y'), ('y');
CREATE TABLE t2(a, b);
INSERT INTO t2 VALUES('x', 1), ('z', 2), ('z', 3);
SELECT 'q1';
SELECT a FROM t1 UNION SELECT a FROM t2 GROUP BY a HAVING count(*)>1 ORDER BY a COLLATE NOCASE;
SELECT 'q2';
SELECT a FROM t1 EXCEPT SELECT a FROM t2 GROUP BY a HAVING count(*)>1 ORDER BY a COLLATE NOCASE;
SELECT 'q3';
SELECT a FROM t1 UNION SELECT b FROM t2 GROUP BY b ORDER BY 1 COLLATE NOCASE;
SELECT 'q4';
SELECT a, 0 FROM t1 UNION SELECT a, count(*) FROM t2 GROUP BY a ORDER BY 1 COLLATE NOCASE;
SELECT 'q5';
SELECT 1 UNION SELECT 2 UNION ALL SELECT DISTINCT 1 ORDER BY 1 COLLATE NOCASE;
