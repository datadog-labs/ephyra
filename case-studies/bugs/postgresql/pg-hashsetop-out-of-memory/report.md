# pgsql-bugs report: hash tables pre-sized from the planner's estimate (19 regression)

Status: submitted 2026-10-15 as BUG #19759, https://www.postgresql.org/message-id/19759-acd3f33402e54f36%40postgresql.org (version field 19beta4).
(forwards to pgsql-bugs). Not a security issue in the sense of the security policy (an ordinary user can make a
backend allocate a lot of memory; no crash beyond OOM handling, no data exposure). It is a regression in
REL_19_BETA1 through REL_19_RC1, so worth sending before the release.

## Form fields

- PostgreSQL version: choose `19beta4` (the newest the form lists; the body names 19rc1 and master)
- Operating system: `Ubuntu 24.04 x86_64 (also macOS 26 arm64)`
- Short description: `Hashed SetOp / RecursiveUnion pre-allocate up to 64 GB from the row estimate since 8f29467c57f`

## Body (plain text)

```
Hello,

Since commit 8f29467c57f ("Change "long" numGroups fields to be Cardinality
(i.e., double)", 2025-11-02), BuildTupleHashTable() no longer limits the
initial size of a TupleHashTable to hash_mem. Its new comment says that
staying within hash_mem is the caller's job. But nodeRecursiveunion.c and
nodeSetOp.c pass the planner's raw group estimate, so the executor now
allocates and zeroes a bucket array of up to 2^32 entries x 16 bytes =
64 GB in ExecInit, before reading a single row, whenever the estimate is
large. PostgreSQL 18 is not affected (it capped nbuckets at
get_hash_memory_limit() / entrysize). The commit is in REL_19_BETA1
through REL_19_RC1.

Two ways to reach it:

1. Recursive UNION, default settings. It can only be hashed, and
   generate_recursion_path() estimates its groups as lrows + 10 * rrows,
   so an inflated estimate goes straight into the allocation. With an
   empty, never-analyzed table:

     CREATE TABLE t0(c0 int4);
     WITH RECURSIVE r(w, x, y, z) AS (
       SELECT a.c0, b.c0, c.c0, d.c0 FROM t0 a, t0 b, t0 c, t0 d
       UNION
       SELECT w, x, y, z FROM r WHERE false)
     SELECT count(*) FROM r;

   On master (49103c3cd63, Ubuntu 24.04, x86_64, the server under
   ulimit -v 6 GB):

     ERROR:  53200: out of memory
     DETAIL:  Failed on request of size 68719476736 in memory context "ExecutorState".
     LOCATION:  MemoryContextAllocationFailure, mcxt.c:1208

   Without a memory limit the backend simply allocates it: on macOS its
   RSS passed 3 GB within a few milliseconds. On 18.6 the query returns 0
   at once.

   An accurate estimate is enough to see the difference, without any
   tables:

     WITH RECURSIVE r(n) AS (
       SELECT g FROM generate_series(1, 1000000) g
       UNION SELECT n + 1 FROM r WHERE n < 0)
     SELECT count(*) FROM r;

   Peak backend RSS: 1086 MB on master, 115 MB on 18.6 (same result).
   With 10 million rows master allocates 8 GB up front, while 18.6 peaks
   at about 1 GB.

2. Hashed set operations whose table the planner itself expects to
   exceed hash_mem. create_setop_path() only marks such a path disabled,
   so it is chosen when the sorted alternative is disabled even more,
   e.g. with enable_sort = off:

     BEGIN;
     SET LOCAL enable_sort = off;
     SELECT a.c0, b.c0, c.c0, d.c0 FROM t0 a, t0 b, t0 c, t0 d
     EXCEPT SELECT 1, 2, 3, 4;
     ROLLBACK;

   EXPLAIN shows "HashSetOp Except ... rows=1600000000 ... Disabled: true".
   On master it tries to allocate 32 GB (2^31 buckets) and fails or
   exhausts memory; on 18.6 it returns no rows in about 2 ms. EXCEPT ALL,
   INTERSECT and INTERSECT ALL behave the same. A small work_mem does
   not help, since the initial size no longer depends on it. HashAggregate
   is not affected (hash_choose_num_buckets() bounds it).

Versions tested:
  - master 158b6d5109f (macOS 26, arm64, Apple clang 21) and 49103c3cd63
    (Ubuntu 24.04, x86_64, gcc 13.3), both --enable-cassert --enable-debug
  - REL_18_STABLE a85b5523e67 (18.6, macOS): not affected
Rechecked on 2026-10-15 (Ubuntu 24.04, x86_64, gcc 13.3, --enable-cassert
--enable-debug, server under ulimit -v 6 GB) on 19rc1
(REL_19_STABLE 0fb192f1b02), current master aa7b9cf132b and
REL_18_STABLE 4d62ef92fda (18.6): on 19rc1 and master the three queries
above fail with requests of 34359738368, 68719476736 and 8589934592
bytes, and the 1M-row recursive UNION peaks at 1088 MB; on 18.6 all
three succeed and that query peaks at 103 MB.

Restoring a cap in BuildTupleHashTable brings master back to 18's
behaviour for all of the above (1M rows: 113 MB; the table still grows on
demand when the input really is large):

--- a/src/backend/executor/execGrouping.c
+++ b/src/backend/executor/execGrouping.c
@@ -216,6 +216,21 @@ BuildTupleHashTable(PlanState *parent,
 	else
 		nbuckets = (uint32) nelements;
 
+	/*
+	 * Limit the initial table size request to not more than hash_mem.  The
+	 * planner can choose a hashed plan it expects to exceed hash_mem (it is
+	 * only "disabled", e.g. when sorting is disabled too), and nodes such as
+	 * RecursiveUnion have no alternative at all; the table can still grow on
+	 * demand if the input really is that large.
+	 */
+	{
+		double		limit = (double) get_hash_memory_limit() /
+			(sizeof(TupleHashEntryData) + MAXALIGN(additionalsize));
+
+		if (nbuckets > limit)
+			nbuckets = (uint32) Max(limit, 1);
+	}
+
 	/* tuplescxt must be separate, else ResetTupleHashTable breaks things */
 	Assert(metacxt != tuplescxt);

Capping in the RecursiveUnion and SetOp callers instead would work too; in
any case the initial size should not exceed hash_mem.

Found with a query fuzzer.

Regards,
<name>
```

## Verification

- Cap removal: `git show 8f29467c57f -- src/backend/executor/execGrouping.c` removes "Limit initial table size
  request to not more than hash_mem" and adds the "not our job" comment; `git tag --contains 8f29467c57f`:
  REL_19_BETA1..RC1 (main session).
- Recursive UNION on an empty table, Linux: the 64 GiB request (68719476736 bytes) and its error text, on the
  workspace's master 49103c3cd63 cluster under ulimit -v 6144 MB (main session, scratch database, dropped).
- Same query on macOS master 158b6d5109: backend RSS passed 3 GB within 8 ms and was killed by a watchdog (main
  session).
- 1M-row recursive UNION, default settings: peak RSS 1086 MB (master) vs 115 MB (18.6), both returning 1000000
  (main session, RSS polled every 20 ms).
- 10M rows, 18.6 vs master; the EXCEPT plans (EXPLAIN: HashSetOp ... Disabled: true, same plans on master and
  18); EXCEPT ALL / INTERSECT [ALL]; work_mem=64kB; HashAggregate bounded; the patched build's figures: triage
  subagent, verify/matrix-master.txt, matrix-18.txt, matrix-fix.txt.
- Not checked: a separate REL_19_STABLE build; pgsql-bugs searched only through the archive's general search (3
  requests, no hit).
