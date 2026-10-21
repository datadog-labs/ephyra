CREATE VIRTUAL TABLE ft USING fts5(a);
INSERT INTO ft(rowid, a) VALUES (1, 'apple'), (2, 'apple'), (3, 'apple pie');

SELECT count(*) FROM ft WHERE ft MATCH 'apple' AND rowid = 2.5;
SELECT count(*) FROM ft WHERE ft MATCH 'apple' AND rowid = 'x';
SELECT group_concat(rowid) FROM ft WHERE ft MATCH 'apple' AND rowid = 2.0;

CREATE TABLE t1(x);
INSERT INTO t1 VALUES (3), (2.5), ('x');
SELECT t1.x, ft.rowid FROM t1 LEFT JOIN ft ON ft.rowid = t1.x AND ft MATCH 'pie';
SELECT count(*) FROM ft WHERE ft MATCH 'apple' AND rowid IN (2.5, 'x');
