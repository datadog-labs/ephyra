-- WITHOUT ROWID table, column added by ALTER TABLE, IN on the primary key,
-- ORDER BY the added column NULLS LAST.
CREATE TABLE t1(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t1 VALUES(1.5);
ALTER TABLE t1 ADD COLUMN b;
PRAGMA integrity_check;                                      -- ok
SELECT * FROM t1 WHERE a IN (1.5, 2) ORDER BY b NULLS LAST;  -- Error: database disk image is malformed; expected 1.5|

CREATE TABLE t2(a PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t2 VALUES(1), (2);
ALTER TABLE t2 ADD COLUMN b DEFAULT 5;
INSERT INTO t2 VALUES(3, NULL);
SELECT * FROM t2 WHERE a IN (1, 2, 3) ORDER BY b NULLS LAST; -- 2|5 3|  (row 1|5 missing); expected 1|5 2|5 3|
