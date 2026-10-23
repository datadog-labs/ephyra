-- create with sqlite3 (writes 128.0 in a REAL column as an integer):
CREATE TABLE t(a REAL, b);
INSERT INTO t VALUES (128.0, 1);
CREATE INDEX i ON t(typeof(a), b);
CREATE TRIGGER g BEFORE UPDATE ON t BEGIN SELECT 1; END;
-- then run with tursodb on that file:
-- UPDATE t SET b = 2;
-- PRAGMA integrity_check;
-- UPDATE t SET b = 3;
-- SELECT a, b FROM t;
