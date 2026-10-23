-- Pre-triage check for sqlite-fts5-not-eof-rhs-instances
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected":
--   q1|[g] b
--   q2|[g] b
-- Buggy builds (trunk 3769f60be38b, 3.46.0 to 3.53.2) print
--   q1|[g] [b]
--   q2|[g] [b]
CREATE VIRTUAL TABLE t1 USING fts5(a);
INSERT INTO t1(rowid, a) VALUES (1, 'g b');
SELECT 'q1', highlight(t1, 0, '[', ']') FROM t1('g OR (zz NOT b)');
CREATE VIRTUAL TABLE t2 USING fts5(a, detail=none);
INSERT INTO t2(rowid, a) VALUES (1, 'g b');
SELECT 'q2', highlight(t2, 0, '[', ']') FROM t2('g OR (zz NOT b)');
