CREATE VIRTUAL TABLE g USING geopoly();
INSERT INTO g(rowid, _shape) VALUES
  (6, '[[0,0],[1,0],[0,1],[0,0]]'), (7, '[[0,0],[1,0],[0,1],[0,0]]');

SELECT count(*) FROM g WHERE rowid = 6.5;
SELECT count(*) FROM g WHERE rowid = '6abc';
SELECT count(*) FROM g WHERE rowid IN (6.5, 7);

CREATE TABLE t1(x);
INSERT INTO t1 VALUES (6), (6.5), ('7abc');
SELECT t1.x, g.rowid FROM t1 LEFT JOIN g ON g.rowid = t1.x;

DELETE FROM g WHERE rowid = 7.5;
SELECT group_concat(rowid) FROM g;
