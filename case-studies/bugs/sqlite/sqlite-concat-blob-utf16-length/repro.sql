-- concat() and concat_ws() take each argument's size with sqlite3_value_bytes()
-- before converting it with sqlite3_value_text(). For a BLOB in a UTF-16
-- database the size is the BLOB's, not the size of the UTF-8 text it converts
-- to, so the result keeps the terminator and bytes past it that were never
-- written (or loses the end of the text when the UTF-8 is longer).
PRAGMA encoding = 'UTF-16be';
SELECT hex(concat(x'0041'));             -- 00410000          expected 0041
SELECT hex(concat(x'00410042'));         -- 0041004200000097  expected 00410042 (the last byte varies: stale memory)
SELECT hex(concat_ws(x'002C', 'a', 'b')); -- 0061002C00000062  expected 0061002C0062
SELECT hex(concat(x'0800'));             -- FFFD (the first 2 of 3 UTF-8 bytes) expected 0800

-- The argument register is converted in place, so a constant BLOB is wrong
-- only for the first row; the answer then depends on the scan order.
CREATE TABLE t1(c0);
INSERT INTO t1 VALUES (NULL), (0);
CREATE INDEX t1_i0 ON t1(c0 DESC);
SELECT hex(concat(x'0041', c0)) FROM t1 NOT INDEXED;
-- 00410000  00410030           expected 0041  00410030
SELECT hex(concat(x'0041', c0)) FROM t1 INDEXED BY t1_i0;
-- 004100000030  0041           expected 00410030  0041
