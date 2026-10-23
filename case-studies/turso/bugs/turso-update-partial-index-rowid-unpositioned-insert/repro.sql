CREATE TABLE t(id INTEGER PRIMARY KEY, a, b);
CREATE INDEX ia ON t(a);
CREATE INDEX ib ON t(b) WHERE id > 10;
INSERT INTO t VALUES (1, 1, 0);
UPDATE t SET b = 2 WHERE a = 1;
SELECT * FROM t;
-- corruption variant (fresh database):
-- CREATE TABLE t(id INTEGER PRIMARY KEY, a, b);
-- CREATE INDEX ia ON t(a);
-- CREATE INDEX ib ON t(b) WHERE id > 1;
-- INSERT INTO t VALUES (2, 1, 0), (1, 2, 0);
-- UPDATE t SET b = 9 WHERE a > 0;
-- SELECT * FROM t;
-- PRAGMA integrity_check;
