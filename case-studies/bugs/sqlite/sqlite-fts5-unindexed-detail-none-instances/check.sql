-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5-unindexed-detail-none-instances
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
CREATE VIRTUAL TABLE t1 USING fts5(a, b UNINDEXED, detail=none);
INSERT INTO t1 VALUES ('one two zeta three four', 'x y zeta');
SELECT snippet(t1, -1, '[', ']', '..', 3) FROM t1('zeta');
SELECT 'q2';
SELECT highlight(t1, 1, '[', ']') FROM t1('zeta');
SELECT 'q3';
CREATE VIRTUAL TABLE t2 USING fts5(a, b UNINDEXED, detail=column);
INSERT INTO t2 VALUES ('one two zeta three four', 'x y zeta');
SELECT snippet(t2, -1, '[', ']', '..', 3) FROM t2('zeta');
SELECT 'q4';
CREATE VIRTUAL TABLE t3 USING fts5(a, b UNINDEXED, detail=full);
INSERT INTO t3 VALUES ('one two zeta three four', 'x y zeta');
INSERT INTO t1 VALUES ('zeta', 'zeta'), ('five', 'six');
INSERT INTO t3 VALUES ('zeta', 'zeta'), ('five', 'six');
SELECT rowid, printf('%.6e', bm25(t1)) FROM t1('zeta');
SELECT 'q5';
SELECT rowid, printf('%.6e', bm25(t3)) FROM t3('zeta');
