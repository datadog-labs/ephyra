CREATE VIRTUAL TABLE h USING fts5(a, b UNINDEXED, locale=1, content='', contentless_unindexed=1);
INSERT INTO h(rowid, a, b) VALUES (4, 'p q', 'r');
SELECT rowid, * FROM h;
