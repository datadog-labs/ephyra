-- A row-value IN (SELECT pk, added FROM t) on a WITHOUT ROWID table, where
-- "added" is a column added by ALTER TABLE ADD COLUMN. sqlite3FindInIndex()
-- picks the PRIMARY KEY b-tree as the IN index, since it holds every table
-- column, and probes it with a key that includes the added column; rows
-- written before the ALTER have no field for it.
CREATE TABLE t1(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t1 VALUES(1.5), (2.5);
ALTER TABLE t1 ADD COLUMN b DEFAULT 5;
PRAGMA integrity_check;
SELECT (1.5, 5) IN (SELECT a, b FROM t1);
-- malformed       expected 1

-- An integer key: no error, a wrong answer.
CREATE TABLE t3(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t3 VALUES(1), (2), (3);
ALTER TABLE t3 ADD COLUMN b DEFAULT 5;
SELECT (2, 5) IN (SELECT a, b FROM t3);
-- NULL            expected 1

-- The IN driving a loop: the added column is read from the index cursor,
-- which yields NULL rather than the column default.
CREATE TABLE t2(x, y);
INSERT INTO t2 VALUES(1.5, 5), (2.5, 6);
CREATE INDEX t2xy ON t2(x, y);
SELECT * FROM t2 WHERE (x, y) IN (SELECT a, b FROM t1);
-- (no rows)       expected 1.5|5
