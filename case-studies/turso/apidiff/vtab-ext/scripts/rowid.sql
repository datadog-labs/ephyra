CREATE VIRTUAL TABLE v USING memvt;
INSERT INTO v(a, b) VALUES (7, 'x');
UPDATE v SET rowid = rowid + 1 WHERE rowid = 1;
SELECT rowid, a, b FROM v;
