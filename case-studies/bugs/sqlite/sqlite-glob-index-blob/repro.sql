-- GLOB/LIKE prefix optimization vs. invalid (over-length) UTF-8 data.
-- SQLite trunk 7b468f6f1d63 (3.54.0, 2026-10-09). Run: sqlite3 :memory: < repro.sql
--
-- x'fba9' is not valid UTF-8. SQLite's decoder accepts over-length
-- sequences (src/utf.c:160), so it reads fb a9 as U+00E9 'é'.
-- The full scan matches it. The index range scan (c >= 'é' AND c < 'ê', run
-- once for TEXT values and once for BLOB values) compares bytes, and
-- fb a9 is outside [c3 a9, c3 aa), so the row is never visited.

CREATE TABLE t(c);
CREATE INDEX ti ON t(c);
INSERT INTO t VALUES(x'fba9');

SELECT count(*) FROM t WHERE c GLOB 'é*';              -- actual: 0
SELECT count(*) FROM t NOT INDEXED WHERE c GLOB 'é*';  -- actual: 1
-- "Expected" (if this were a bug): both 1. Per https://sqlite.org/invalidutf.html
-- (garbage in, garbage out) neither answer is guaranteed.

EXPLAIN QUERY PLAN SELECT * FROM t WHERE c GLOB 'é*';
-- `--SEARCH t USING COVERING INDEX ti (c>? AND c<?)

-- Controls: valid input agrees between the two plans.
INSERT INTO t VALUES(x'c3a9'), (x'616263'), ('abc');
SELECT count(*) FROM t WHERE c GLOB 'ab*';             -- 2 (blob and text)
SELECT count(*) FROM t NOT INDEXED WHERE c GLOB 'ab*'; -- 2
SELECT count(*) FROM t WHERE c GLOB 'é*';              -- 1 (x'c3a9'; x'fba9' missed)
SELECT count(*) FROM t NOT INDEXED WHERE c GLOB 'é*';  -- 2
