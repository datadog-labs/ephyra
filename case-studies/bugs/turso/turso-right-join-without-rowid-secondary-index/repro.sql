-- Step 1: create the file with SQLite (Turso rejects CREATE INDEX on WITHOUT ROWID tables):
--   sqlite3 w.db < repro.sql   (runs only the statements up to the query)
-- Step 2: run the last SELECT with tursodb on w.db:
--   echo 'SELECT count(*) FROM t0 FULL JOIN t1 ON NULL;' | tursodb -q -m list w.db
CREATE TABLE t0(c0);
CREATE TABLE t1(c0 PRIMARY KEY, c1) WITHOUT ROWID;
CREATE INDEX t1_i0 ON t1(c1);
-- query for tursodb:
-- SELECT count(*) FROM t0 FULL JOIN t1 ON NULL;
