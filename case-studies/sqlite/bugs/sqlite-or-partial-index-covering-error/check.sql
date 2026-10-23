-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-or-partial-index-covering-error
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t(a INT, b);
CREATE INDEX i ON t(b) WHERE a=5;
SELECT 'q1';
SELECT * FROM t WHERE b=0 AND a=5 AND a=1 OR rowid=0;
INSERT INTO t VALUES(5,0),(1,0),(5,1);
SELECT 'q2';
SELECT rowid, a, b FROM t WHERE b=0 AND a=5 AND a>1 OR rowid=2;
CREATE TABLE t0(c0 INTEGER) STRICT;
CREATE INDEX t0_i0 ON t0(CAST(c0 AS INTEGER));
CREATE INDEX t0_i1 ON t0((c0 + 1)) WHERE (c0 = 3.5);
SELECT 'q3';
SELECT count(*) FROM t0 AS a0 JOIN t0 AS a1 ON ((a0.c0 + 1) BETWEEN (NOT ((a1.c0 LIKE '😀é') AND -315)) AND 0) WHERE ((((a0.c0 IN (SELECT a0.c0 FROM t0 AS b0 WHERE abs(CAST('É' AS TEXT)))) AND ((a0.c0 <= 16) AND (a0.c0 = 3.5))) AND ((a1.c0 <= 16) AND (a1.c0 = 3.5))) OR (CAST(a0.c0 AS INTEGER) <= 0));
