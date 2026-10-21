-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5-tokendata-missing-term-dlidx-corrupt
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
CREATE VIRTUAL TABLE f USING fts5(a, tokendata=1);
INSERT INTO f(f, rank) VALUES('pgsz', 64);
WITH s(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM s WHERE i<100)
INSERT INTO f(rowid, a) SELECT i, iif(i<100, 'u0', 'v') FROM s;
SELECT count(*) FROM f('u1');
SELECT 'q2';
INSERT INTO f(rowid, a) VALUES(200, 'u1');
SELECT rowid FROM f('u1');
SELECT 'q3';
CREATE VIRTUAL TABLE g USING fts5(a, tokendata=1);
WITH s(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM s WHERE i<10000)
INSERT INTO g(rowid, a) SELECT i, iif(i<10000, 'u0', 'v') FROM s;
SELECT count(*) FROM g('u1');
SELECT 'q4';
INSERT INTO f(f) VALUES('integrity-check');
INSERT INTO g(g) VALUES('integrity-check');
SELECT (SELECT count(*) FROM f('u0')), (SELECT count(*) FROM g('v'));
