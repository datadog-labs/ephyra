-- VACUUM FULL / CLUSTER / CREATE INDEX: "missing chunk number 0 for toast value"
-- Run: psql -X -d postgres -f minimal.sql   (PGHOST/PGPORT/PGUSER in the environment;
-- psql must be on PATH for the \! line). autovacuum off is not required but keeps it quiet.
-- Needs one other session, in ANOTHER database, holding an XID assigned before the DELETE.
\set ON_ERROR_STOP 0
DROP DATABASE IF EXISTS toast_other;
CREATE DATABASE toast_other;
DROP DATABASE IF EXISTS toast_repro;
CREATE DATABASE toast_repro;
\c toast_repro
CREATE TABLE t (id int, b text STORAGE EXTERNAL);   -- 20 rows: one heap page, mostly empty (never pruned on access)
CREATE TABLE u (x int);                              -- unrelated table
INSERT INTO t SELECT g, repeat(md5(g::text), 300) FROM generate_series(1, 20) g;  -- 9600-byte values, 5 chunks each
-- session 2, other database: holds xid P for 10 s
\! psql -X -d toast_other -Atc "SELECT pg_current_xact_id(), pg_sleep(10)" >/dev/null &
SELECT pg_sleep(1);
DELETE FROM t WHERE id % 2 = 0;          -- xid D > P
VACUUM u;                                -- lazy VACUUM skips its own xmin: this backend's GlobalVisDataRels.maybe_needed jumps past D
SELECT count(*), sum(length(b)) FROM t;  -- detoasting live rows prunes the toast pages: chunks deleted by D are removed
CREATE EXTENSION IF NOT EXISTS amcheck;
SELECT * FROM verify_heapam('t', check_toast => true);  -- reports "toast value .. not found" for the 10 deleted rows
VACUUM FULL t;                           -- ERROR:  missing chunk number 0 for toast value N in pg_toast_M (XX001)
SELECT pg_sleep(10);                     -- once session 2's xid ends:
VACUUM FULL t;                           -- succeeds
SELECT count(*), sum(length(b)) FROM t;  -- 10 | 96000: no live data lost
