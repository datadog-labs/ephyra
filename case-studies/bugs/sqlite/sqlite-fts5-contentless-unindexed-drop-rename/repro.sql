CREATE VIRTUAL TABLE h USING fts5(a, b UNINDEXED, content='', contentless_unindexed=1);
INSERT INTO h(rowid, a, b) VALUES (1, 'x y', 'b1');
ALTER TABLE h RENAME TO h2;
SELECT name FROM sqlite_schema ORDER BY name;
INSERT INTO h2(rowid, a, b) VALUES (2, 'x z', 'b2');
DROP TABLE h2;
SELECT name FROM sqlite_schema;
