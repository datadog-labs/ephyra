# Candidate: TupleHashTable pre-sized from the planner's estimate (PostgreSQL 19 regression)

Found 2026-10-14 by the PostgreSQL fuzz runner on workspace-ephyra-fuzz-pg (plans-hashagg variant,
`out of memory (53200)` under the workspace's 6 GB RLIMIT_AS), triaged the same day.

Commit 8f29467c57f (2025-11-02, "Change "long" numGroups fields to be Cardinality") removed
BuildTupleHashTable's cap of the initial size at hash_mem; its new comment says enforcing hash_mem is
the caller's job. nodeSetOp.c and nodeRecursiveunion.c pass the raw planner estimate, so the executor
allocates and zeroes up to 2^32 buckets x 16 B = 64 GiB in ExecInit, before reading a row.
In REL_19_BETA1 through REL_19_RC1 and master; REL_18_STABLE (18.6) is not affected.

- (A) a hashed set operation the planner marks `Disabled: true` but picks with enable_sort = off
  (the fuzzer's shape: an empty, unanalyzed table, EXCEPT ALL over a 4-way join);
- (B) a recursive UNION with default settings (it can only be hashed; estimate lrows + 10*rrows);
- (C) default settings, accurate estimate: a 1M-row recursive UNION peaks at 1086 MB on master vs
  115 MB on 18.6 (rechecked in the main session); 10M rows allocate 8 GiB up front on master.

Restoring the cap (verify/fix-clamp.patch) brings master back to 18's figures. No report or fix found
upstream (3 archive searches). Not reported. Files: minimal.sql, verify/.
