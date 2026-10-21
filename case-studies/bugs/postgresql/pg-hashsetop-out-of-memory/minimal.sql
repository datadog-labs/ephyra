-- pg-hashsetop-out-of-memory: minimal reproducers (PostgreSQL master / 19, since commit 8f29467c57f).
-- Since 8f29467c57f, BuildTupleHashTable no longer limits its initial size to hash_mem, so a
-- TupleHashTable is pre-sized from the planner's group estimate: up to 2^32 buckets x 16 B = 64 GiB,
-- allocated and zeroed in ExecInit, before reading a single row.
-- Under RLIMIT_AS (ulimit -v) this fails with "out of memory"; without a limit it eats RAM.
-- REL_18_STABLE (18.6) is unaffected (it caps the initial size at hash_mem / entrysize).
-- WARNING: run under a memory watchdog (verify/wd.py) or `ulimit -v` on Linux.

CREATE TABLE t0(c0 int4);   -- empty, never analyzed: 2550 rows per table, 200 distinct values assumed

-- (A) the fuzzer's shape: a hashed set op the planner itself marks "Disabled: true" (too big for
--     hash_mem), chosen anyway because the sorted alternative has more disabled nodes.
BEGIN;
SET LOCAL enable_sort = off;
-- estimated groups 200^4 = 1.6e9 -> 2^31 buckets -> 32 GiB up-front allocation
SELECT a.c0, b.c0, c.c0, d.c0 FROM t0 a, t0 b, t0 c, t0 d EXCEPT SELECT 1, 2, 3, 4;
ROLLBACK;

-- (B) default settings, no hints: RecursiveUnion has no non-hashed alternative and its group estimate
--     is lrows + 10 * rrows (here 4.2e13 -> clamped to 2^32 buckets -> 64 GiB).
WITH RECURSIVE r(w, x, y, z) AS (
  SELECT a.c0, b.c0, c.c0, d.c0 FROM t0 a, t0 b, t0 c, t0 d
  UNION
  SELECT w, x, y, z FROM r WHERE false)
SELECT count(*) FROM r;

-- (C) default settings, no tables, accurate estimate: 10M-row recursive UNION.
--     18.6: peak about 0.98 GB RSS, finishes. master: allocates 8 GiB up front (killed above 3 GB at 0.18 s).
--     With 1M rows: 18.6 115 MB vs master 1084 MB.
WITH RECURSIVE r(n) AS (SELECT g FROM generate_series(1, 10000000) g UNION SELECT n + 1 FROM r WHERE n < 0)
SELECT count(*) FROM r;
