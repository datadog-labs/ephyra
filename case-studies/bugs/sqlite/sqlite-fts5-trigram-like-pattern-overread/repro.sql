-- Run with an ASan build of trunk and lookaside off:
--   ./sqlite3 -lookaside 0 0 :memory: < repro.sql
-- The 7-byte LIKE pattern's OP_String8 copy is exactly 8 bytes, so the read of byte 8 is reported.
CREATE VIRTUAL TABLE t USING fts5(a, tokenize=trigram);
SELECT * FROM t WHERE a LIKE 'abcdefg';
