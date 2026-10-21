-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-temp-rename-column-twice
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
CREATE TEMP TABLE t(a);
CREATE TEMP TRIGGER tr BEFORE DELETE ON t WHEN old.a > 0 BEGIN SELECT 1; END;
ALTER TABLE t RENAME COLUMN a TO b;
SELECT sql FROM sqlite_temp_schema ORDER BY name;
SELECT 'q2';
CREATE TEMP TABLE t2(a);
CREATE TEMP VIEW v2 AS SELECT a FROM t2;
ALTER TABLE t2 RENAME COLUMN a TO b;
SELECT sql FROM sqlite_temp_schema WHERE name IN ('t2', 'v2') ORDER BY name;
SELECT 'q3';
CREATE TABLE t1(x);
CREATE TEMP TABLE t5(a, b);
CREATE TEMP TRIGGER w BEFORE INSERT ON t1 BEGIN UPDATE t5 SET b = 1 WHERE b > 0; END;
ALTER TABLE t5 RENAME COLUMN b TO c;
SELECT sql FROM sqlite_temp_schema WHERE name IN ('t5', 'w') ORDER BY name;
