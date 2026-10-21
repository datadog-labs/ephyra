-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-drop-not-null-not-deferrable
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Needs a build with ALTER TABLE ... ALTER COLUMN ... DROP NOT NULL (3.53.0 or later).
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream:
--   q1|CREATE TABLE c1(x REFERENCES p NOT DEFERRABLE, y)
--   q2|CREATE TABLE c2(x REFERENCES p DEFERRABLE INITIALLY DEFERRED, y)
--   q3|CREATE TABLE c3(x REFERENCES p DEFERRABLE INITIALLY DEFERRED, y)
--   q4|CREATE TABLE c4(x REFERENCES p DEFERRABLE INITIALLY DEFERRED NOT NULL, y)
-- Buggy builds (3.53.2, 3.53.4, 3.54.0, trunk 3769f60be38b) print
-- "q1|CREATE TABLE c1(x REFERENCES p NOT NULL, y)" and lose the
-- "DEFERRABLE INITIALLY DEFERRED" in q2, q3 and q4.
CREATE TABLE p(a PRIMARY KEY);
CREATE TABLE c1(x REFERENCES p NOT DEFERRABLE NOT NULL, y);
CREATE TABLE c2(x REFERENCES p NOT NULL DEFERRABLE INITIALLY DEFERRED, y);
CREATE TABLE c3(x REFERENCES p CONSTRAINT c CHECK(x>0) DEFERRABLE INITIALLY DEFERRED, y);
CREATE TABLE c4(x REFERENCES p NOT NULL DEFERRABLE INITIALLY DEFERRED, y);
ALTER TABLE c1 ALTER x DROP NOT NULL;
ALTER TABLE c2 ALTER x DROP NOT NULL;
ALTER TABLE c3 DROP CONSTRAINT c;
ALTER TABLE c4 ALTER x SET NOT NULL;
SELECT 'q1', sql FROM sqlite_schema WHERE name='c1';
SELECT 'q2', sql FROM sqlite_schema WHERE name='c2';
SELECT 'q3', sql FROM sqlite_schema WHERE name='c3';
SELECT 'q4', sql FROM sqlite_schema WHERE name='c4';
