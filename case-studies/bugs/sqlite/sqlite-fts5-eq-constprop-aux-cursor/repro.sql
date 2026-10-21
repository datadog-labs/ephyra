CREATE VIRTUAL TABLE ft USING fts5(a);
INSERT INTO ft(rowid, a) VALUES (1, 'one apple'), (2, 'apple pie');

SELECT rowid FROM ft WHERE ft = 'apple' AND snippet(ft, 0, '[', ']', '..', 4) IS NOT NULL;
SELECT rowid FROM ft WHERE ft = 'apple' AND bm25(ft) < 0;
SELECT count(*) FROM ft WHERE ft = 'apple' AND ft = 'pie';
