-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5vocab-term-range-utf16-order
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
-- Buggy builds print "q1|apple", "q2|ĳs" and "q3|apple b ĳs" (UTF-8 order).
PRAGMA encoding = 'UTF-16le';
CREATE VIRTUAL TABLE ft USING fts5(a);
INSERT INTO ft VALUES ('apple ' || char(0x133) || 's b');
CREATE VIRTUAL TABLE v USING fts5vocab(ft, 'row');
SELECT 'q1', group_concat(term, ' ') FROM v WHERE term < 'b';
SELECT 'q2', group_concat(term, ' ') FROM v WHERE term >= char(0x133, 0x73);
SELECT 'q3', group_concat(term, ' ') FROM (SELECT term FROM v ORDER BY term);
SELECT 'q4', group_concat(term, ' ') FROM v WHERE term = char(0x133, 0x73);
