-- Pre-triage check: sqlite-concat-blob-utf16-length
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the expected one;
-- trunk printing exactly that means the bug was fixed upstream.
PRAGMA encoding = 'UTF-16be';
CREATE TABLE t1(c0);
INSERT INTO t1 VALUES (NULL), (0);
CREATE INDEX t1_i0 ON t1(c0 DESC);
SELECT 'q1';
SELECT hex(concat(x'0041')), hex(concat(x'00410042')), hex(concat(x'0800'));
SELECT 'q2';
SELECT hex(concat_ws(x'002C', 'a', 'b')), hex(concat_ws('-', x'0041', x'0042'));
SELECT 'q3';
SELECT hex(concat(x'0041', c0)) FROM t1 NOT INDEXED;
SELECT 'q4';
SELECT hex(concat(x'0041', c0)) FROM t1 INDEXED BY t1_i0;
