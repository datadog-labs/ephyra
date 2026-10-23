PRAGMA encoding = 'UTF-16le';
CREATE VIRTUAL TABLE ft USING fts5(a);
INSERT INTO ft VALUES ('apple ĳs b');   -- ĳ is U+0133
CREATE VIRTUAL TABLE v USING fts5vocab(ft, 'row');
CREATE TABLE t(term TEXT);
INSERT INTO t SELECT term FROM v;

SELECT term FROM v WHERE term < 'b';
SELECT term FROM v WHERE +term < 'b';
SELECT term FROM v ORDER BY term;
SELECT term FROM t ORDER BY term;
