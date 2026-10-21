CREATE VIRTUAL TABLE f USING fts5(a, tokendata=1);
INSERT INTO f(f, rank) VALUES('pgsz', 64);
WITH s(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM s WHERE i<100)
INSERT INTO f(rowid, a) SELECT i, iif(i<100, 'u0', 'v') FROM s;
SELECT count(*) FROM f('u1');
