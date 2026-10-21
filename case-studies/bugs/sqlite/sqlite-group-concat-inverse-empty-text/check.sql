-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-group-concat-inverse-empty-text
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
CREATE TABLE t1(a INTEGER PRIMARY KEY, b);
INSERT INTO t1 VALUES(1, ''), (2, ''), (3, '');
CREATE TABLE t2(a INTEGER PRIMARY KEY, b);
INSERT INTO t2 VALUES(1, 'a'), (2, ''), (3, 'b');
CREATE TABLE t3(a INTEGER PRIMARY KEY, b);
INSERT INTO t3 VALUES(1, 'a'), (2, ''), (3, ''), (4, 'b');
SELECT 'q1';
SELECT quote(group_concat(b) OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)) FROM t1;
SELECT 'q2';
SELECT quote(group_concat(b, '-') OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)) FROM t2;
SELECT 'q3';
SELECT quote(string_agg(b, ' | ') OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)) FROM t3;
SELECT 'q4';
SELECT quote(group_concat(b, '-') OVER (ORDER BY a ROWS 1 PRECEDING)) FROM t3;
SELECT 'q5';
SELECT quote(group_concat(b, CASE a % 2 WHEN 0 THEN '-' ELSE '++' END)
             OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)) FROM t3;
