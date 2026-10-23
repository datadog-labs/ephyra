CREATE VIRTUAL TABLE ft USING fts5(x);
INSERT INTO ft VALUES('apple banana cherry');
CREATE VIRTUAL TABLE v USING fts5vocab(ft, 'row');
CREATE TABLE t(a);
INSERT INTO t VALUES('cherry');

SELECT term FROM v WHERE term = 'banana' OR term < 'c';
SELECT t.a, v.term FROM t RIGHT JOIN v ON v.term = t.a;
