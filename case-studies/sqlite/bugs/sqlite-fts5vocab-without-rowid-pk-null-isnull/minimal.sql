CREATE VIRTUAL TABLE ft USING fts5(a, detail=none);
INSERT INTO ft VALUES('x');
CREATE VIRTUAL TABLE v USING fts5vocab(ft, 'col');
SELECT term, quote(col) FROM v;
SELECT count(*) FROM v WHERE col IS NULL;
SELECT count(*) FROM v WHERE col IS NOT NULL;
