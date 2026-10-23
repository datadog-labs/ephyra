-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-right-join-using-star-ambiguous
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- There is no fix to build: the registry's "expected" is the output of these
-- queries with the * written out by hand, which trunk answers (q1:
-- coalesce(t0.c0, t1.c0), t0.c1, t1.c1, t2.* FROM t0 FULL JOIN t1 ON
-- t0.c0 = t1.c0 ...; q2: t1.c0, ... RIGHT JOIN ...; q3: t0.c0, ... JOIN ...
-- RIGHT JOIN t0 AS t2 ON 1), the rows DuckDB gives for q1 (forum-post.md).
-- Trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t0(c0, c1);
CREATE TABLE t1(c0, c1);
INSERT INTO t0 VALUES (1, 2), (4, 5);
INSERT INTO t1 VALUES (1, 3), (6, 7);
SELECT 'q1';
SELECT * FROM t0 FULL JOIN t1 USING (c0) CROSS JOIN t0 AS t2 ORDER BY 1, 2, 3, 4, 5;
SELECT 'q2';
SELECT * FROM t0 RIGHT JOIN t1 USING (c0) CROSS JOIN t0 AS t2 ORDER BY 1, 2, 3, 4, 5;
SELECT 'q3';
SELECT * FROM t0 JOIN t1 USING (c0) RIGHT JOIN t0 AS t2 ON 1 ORDER BY 1, 2, 3, 4, 5;
