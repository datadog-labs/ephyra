CREATE TABLE t(x);
INSERT INTO t(rowid, x) VALUES (100, 1);
CREATE VIRTUAL TABLE v USING memvt;
INSERT INTO v(a, b) VALUES ('k1', 'x');
SELECT changes(), total_changes(), last_insert_rowid();
UPDATE v SET b = 'y';
SELECT changes(), total_changes(), last_insert_rowid();
DELETE FROM v;
SELECT changes(), total_changes(), last_insert_rowid();
