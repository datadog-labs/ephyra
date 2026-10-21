CREATE VIRTUAL TABLE g USING geopoly(a);
INSERT INTO g(rowid, _shape, a) VALUES
  (1, '[[0,0],[1,0],[0,1],[0,0]]', 'x'),
  (2, '[[5,5],[6,5],[5,6],[5,5]]', 'y');

UPDATE g SET rowid = 7 WHERE rowid = 1;
UPDATE g SET rowid = 8, _shape = _shape WHERE rowid = 2;
SELECT rowid, a FROM g ORDER BY rowid;
