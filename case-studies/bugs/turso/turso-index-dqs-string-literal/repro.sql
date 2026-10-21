-- 1. Run in tursodb:
CREATE TABLE t(c);
CREATE INDEX i1 ON t(datetime(c, "+1 month"));
CREATE INDEX i2 ON t(c) WHERE c < "-61";
SELECT "-61";
-- 2. A database made by sqlite3 3.50.4:
--   sqlite3 a.db 'CREATE TABLE t(c); CREATE INDEX i1 ON t(datetime(c, "+1 month"));'
--   sqlite3 b.db 'CREATE TABLE t(c); CREATE INDEX i2 ON t(c) WHERE c < "-61";'
-- then: tursodb a.db 'SELECT 1;'   and   tursodb b.db 'INSERT INTO t VALUES (1);'
