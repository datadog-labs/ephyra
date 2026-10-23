CREATE VIRTUAL TABLE t1 USING fts5(a, b UNINDEXED, tokenize=trigram);
INSERT INTO t1 VALUES('xyz', 'abcdef');
SELECT count(*) FROM t1 WHERE b LIKE '%bcd%';
SELECT count(*) FROM t1 WHERE b GLOB '*bcd*';
SELECT count(*) FROM t1 WHERE +b LIKE '%bcd%';
