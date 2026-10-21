-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-alter-default-blob-encoding
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
PRAGMA encoding = 'UTF-16le';
CREATE TABLE t1(a); INSERT INTO t1 VALUES(1);
ALTER TABLE t1 ADD COLUMN b DEFAULT x'9e2e';
ALTER TABLE t1 ADD COLUMN c DEFAULT x'0b0b';
SELECT 'q1';
SELECT unicode(CAST(b AS TEXT)), length(CAST(b AS TEXT)) FROM t1;
SELECT 'q2';
SELECT CAST(b AS TEXT) = CAST(x'9e2e' AS TEXT), CAST(c AS TEXT) = CAST(x'0b0b' AS TEXT) FROM t1;
SELECT 'q3';
SELECT unicode(min(CAST(c AS TEXT))) FROM t1;
