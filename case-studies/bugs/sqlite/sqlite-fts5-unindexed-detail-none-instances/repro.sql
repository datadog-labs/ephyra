CREATE VIRTUAL TABLE t1 USING fts5(a, b UNINDEXED, detail=none);
INSERT INTO t1 VALUES ('one two zeta three four', 'x y zeta');

SELECT snippet(t1, -1, '[', ']', '..', 3) FROM t1('zeta');
SELECT highlight(t1, 1, '[', ']') FROM t1('zeta');
