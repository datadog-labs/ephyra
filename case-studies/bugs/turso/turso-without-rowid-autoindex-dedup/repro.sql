-- tursodb -q -m list --experimental-without-rowid :memory: < repro.sql
CREATE TABLE w(k PRIMARY KEY, v) WITHOUT ROWID;
INSERT INTO w VALUES('a',1),('b',1);
CREATE TABLE t(x);
INSERT INTO t VALUES(1);
SELECT t.x FROM t JOIN w ON w.v = t.x;
SELECT count(*) FROM t JOIN w ON w.v = t.x;
SELECT count(*) OVER () FROM t JOIN w ON w.v = t.x;
SELECT w.k FROM t JOIN w ON w.v = t.x;
