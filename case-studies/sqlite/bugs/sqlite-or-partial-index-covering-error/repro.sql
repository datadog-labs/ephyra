-- A term of an OR (MULTI-INDEX OR) that uses a partial index "WHERE a=5"
-- and also reads column a: sqlite3_prepare() fails with
-- "internal query planner error" (SQLITE_INTERNAL) since 3.46.1.
-- No database file needed.

CREATE TABLE t(a INT, b);
CREATE INDEX i ON t(b) WHERE a=5;

-- Empty table: expected no rows; trunk: internal query planner error
SELECT * FROM t WHERE b=0 AND a=5 AND a=1 OR rowid=0;

-- With rows: expected 1|5|0 and 2|1|0 (what NOT INDEXED and 3.44.0-3.46.0 return)
INSERT INTO t VALUES(5,0),(1,0),(5,1);
SELECT rowid, a, b FROM t WHERE b=0 AND a=5 AND a>1 OR rowid=2;
SELECT rowid, a, b FROM t NOT INDEXED WHERE b=0 AND a=5 AND a>1 OR rowid=2;

-- The fuzzer's original (run-0438), on an empty STRICT table: expected 0
CREATE TABLE t0(c0 INTEGER) STRICT;
CREATE INDEX t0_i0 ON t0(CAST(c0 AS INTEGER));
CREATE INDEX t0_i1 ON t0((c0 + 1)) WHERE (c0 = 3.5);
SELECT count(*) FROM t0 AS a0 JOIN t0 AS a1 ON ((a0.c0 + 1) BETWEEN (NOT ((a1.c0 LIKE '😀é') AND -315)) AND 0) WHERE ((((a0.c0 IN (SELECT a0.c0 FROM t0 AS b0 WHERE abs(CAST('É' AS TEXT)))) AND ((a0.c0 <= 16) AND (a0.c0 = 3.5))) AND ((a1.c0 <= 16) AND (a1.c0 = 3.5))) OR (CAST(a0.c0 AS INTEGER) <= 0));
