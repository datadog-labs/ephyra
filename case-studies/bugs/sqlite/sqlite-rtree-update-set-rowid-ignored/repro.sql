CREATE VIRTUAL TABLE r USING rtree(id, x1, x2);
INSERT INTO r VALUES (1, 0, 1), (2, 2, 3);

UPDATE r SET rowid = 9 WHERE id = 2;
SELECT changes();
SELECT * FROM r ORDER BY id;

UPDATE r SET rowid = 8, x2 = 7 WHERE id = 1;
SELECT * FROM r ORDER BY id;

INSERT INTO r(rowid, x1, x2) VALUES (5, 4, 5);
SELECT * FROM r ORDER BY id;
