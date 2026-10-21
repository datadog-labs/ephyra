# Candidate: CLUSTER / REPACK silently loses a row updated in a rolled-back subtransaction

Found 2026-10-14 by the workspace fuzz runner (ephyra-fuzz-pg, model property: `DELETE FROM t1` wrote 0
rows, the model expected 1). Silent loss of committed data, in every version checked: master 158b6d5109,
18.6 (REL_18_STABLE a85b5523e67), 14.24 (built by the triage); REL_19 has the same code (not built).

minimal.sql: a row inserted by the current transaction, updated (HOT) inside a savepoint that is rolled
back, then `CLUSTER t1 USING <btree index>` in the same transaction: the row is gone, before and after
COMMIT. Needs the index-scan CLUSTER path (or REPACK, also its seqscan form on master); the seqscan+sort
path keeps the row, so whether a given table shows it depends on the planner's estimate.

Cause (triage): HeapTupleSatisfiesVacuumHorizon's "deleting subtransaction must have aborted" branch
returns HEAPTUPLE_INSERT_IN_PROGRESS without setting HEAP_XMAX_INVALID; rewrite_heap_tuple then parks the
live tuple as an unresolved chain member, and rewrite_heap_dead_tuple frees it when the aborted new
version turns out dead. One-line fix (set the hint bit there): triage/fix-htsv-xmax-invalid.patch,
make check 239/239. No report found (2 searches).

Correction: the first note here (2026-10-14) said "master regression, 18.6 keeps the row, needs a
leading SAVEPOINT s0". Wrong: the 18.6 comparison ran with autovacuum on, which changed CLUSTER's plan
to seqscan+sort. With the same plan every version loses the row, and a top-level insert suffices.
Rechecked in the main session: minimal.sql gives 0, 0 on 18.6 (default settings) and on master.
Not reported.
