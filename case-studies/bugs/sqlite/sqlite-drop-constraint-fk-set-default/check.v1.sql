-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-drop-constraint-fk-set-default
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Needs a build with ALTER TABLE ... DROP CONSTRAINT on foreign keys (3.54.0 or later).
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream:
--   q1|CREATE TABLE c1(x, y)
--   q2|CREATE TABLE c2(w REFERENCES p DEFERRABLE INITIALLY DEFERRED, x, y)
--   q3|CREATE TABLE c3(x, y)
--   q4|CREATE TABLE c4(x REFERENCES p NOT DEFERRABLE, y)
-- Buggy builds (3.54.0, trunk 3769f60be38b) print an error for c1 and then
-- "q1|CREATE TABLE c1(x CONSTRAINT f ... SET DEFAULT ...)", "q2|... x NOT DEFERRABLE, y)",
-- "q3|CREATE TABLE c3(x, y, generated)" and "q4|CREATE TABLE c4(x REFERENCES p NOT NULL, y)".
CREATE TABLE p(a PRIMARY KEY);
CREATE TABLE c1(x CONSTRAINT f REFERENCES p(a) ON DELETE SET DEFAULT, y);
CREATE TABLE c2(w REFERENCES p DEFERRABLE INITIALLY DEFERRED, x CONSTRAINT f REFERENCES p NOT DEFERRABLE, y);
CREATE TABLE c3(x, y, CONSTRAINT f FOREIGN KEY(x) REFERENCES p MATCH generated);
CREATE TABLE c4(x REFERENCES p NOT DEFERRABLE NOT NULL, y);
ALTER TABLE c1 DROP CONSTRAINT f;
ALTER TABLE c2 DROP CONSTRAINT f;
ALTER TABLE c3 DROP CONSTRAINT f;
ALTER TABLE c4 ALTER x DROP NOT NULL;
SELECT 'q1', sql FROM sqlite_schema WHERE name='c1';
SELECT 'q2', sql FROM sqlite_schema WHERE name='c2';
SELECT 'q3', sql FROM sqlite_schema WHERE name='c3';
SELECT 'q4', sql FROM sqlite_schema WHERE name='c4';
