CREATE TABLE t(x INTEGER);
INSERT INTO t VALUES (-1), (1), (2), (7), (0), (NULL), (0);
SELECT 1 FROM t a, t b JOIN (SELECT 1 FROM t c JOIN t d ON c.x >= d.x) e ON nullif(b.x, a.x) IS NULL AND a.x IS NOT NULL WHERE NOT EXISTS (SELECT 1);
