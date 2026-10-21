-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-overflow-text-bom
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
PRAGMA encoding = 'UTF-16le';
CREATE TABLE t(n, c);
CREATE INDEX i ON t(c);
INSERT INTO t VALUES (1000, char(0xFEFF) || printf('%.*c', 1000, 'a'));
INSERT INTO t VALUES (5000, char(0xFEFF) || printf('%.*c', 5000, 'a'));
SELECT 'q1';
SELECT n, length(c), unicode(c) FROM t NOT INDEXED ORDER BY n;
SELECT 'q2';
SELECT n, length(c), unicode(c) FROM t INDEXED BY i ORDER BY n;
SELECT 'q3';
PRAGMA integrity_check;
