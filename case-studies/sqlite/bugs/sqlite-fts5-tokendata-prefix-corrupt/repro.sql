CREATE VIRTUAL TABLE t1 USING fts5(x, tokendata=1, prefix=1);
INSERT INTO t1 VALUES('vv aa');
BEGIN;
INSERT INTO t1 VALUES('vv aa');
SELECT rowid FROM t1 WHERE t1 MATCH 'v* AND aa';
