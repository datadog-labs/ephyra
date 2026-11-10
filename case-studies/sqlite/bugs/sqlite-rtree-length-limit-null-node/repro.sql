CREATE VIRTUAL TABLE rt USING rtree(id, x0, x1);
INSERT INTO rt VALUES(1, 0, 10);
.limit length 100
INSERT INTO rt VALUES(2, 5, 6);
.limit length 1000000000
SELECT nodeno, typeof(data) FROM rt_node;
SELECT * FROM rt;
PRAGMA integrity_check;
