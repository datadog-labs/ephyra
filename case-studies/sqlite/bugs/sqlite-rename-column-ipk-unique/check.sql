-- Pre-triage check: sqlite-rename-column-ipk-unique
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Correct build (fix.diff applied) prints:
--   q1|CREATE TABLE t1(z INTEGER, PRIMARY KEY(z), UNIQUE(z)) WITHOUT ROWID
--   q2|CREATE TABLE t2(z INTEGER, d, UNIQUE(d, z), PRIMARY KEY(z))
-- Buggy builds (3.25.0 .. 3.54.0, trunk 3769f60be38b, 9eafe6158d10) print
-- "error in table t1 after rename: no such column: c" (same for t2) and the
-- unchanged schemas "q1|CREATE TABLE t1(c INTEGER, ...", "q2|CREATE TABLE t2(c INTEGER, ...".
CREATE TABLE t1(c INTEGER, PRIMARY KEY(c), UNIQUE(c)) WITHOUT ROWID;
CREATE TABLE t2(c INTEGER, d, UNIQUE(d, c), PRIMARY KEY(c));
ALTER TABLE t1 RENAME COLUMN c TO z;
ALTER TABLE t2 RENAME COLUMN c TO z;
SELECT 'q1', sql FROM sqlite_schema WHERE name='t1';
SELECT 'q2', sql FROM sqlite_schema WHERE name='t2';
