# pgsql-bugs report: CLUSTER silently drops a row updated in a rolled-back subtransaction

Status: submitted 2026-10-15 as BUG #19760, https://www.postgresql.org/message-id/19760-8f9a0eba17bc8c92@postgresql.org (replies 2026-10-16 by Takeshi Ideriha and Ayush Tiwari).
(forwards to pgsql-bugs). Silent loss of committed data in every supported branch; not a security issue in the
policy's sense (needs the transaction's own CLUSTER, no privilege boundary crossed).

## Form fields

- PostgreSQL version: choose `18.6` (the body names 19rc1, master and 14.24)
- Operating system: `macOS 26 arm64 (also seen on Ubuntu 24.04 x86_64)`
- Short description: `CLUSTER loses a row the transaction inserted and then updated in a rolled-back subtransaction`

## Body (plain text)

```
Hello,

CLUSTER (and REPACK on master) silently drops a live row when the same
transaction inserted the row and then updated it inside a subtransaction
that was rolled back. No error is raised; the row is gone after COMMIT.

  CREATE TABLE t1(c0 int, c1 int);
  CREATE INDEX t1_i0 ON t1 (c1);
  BEGIN;
  INSERT INTO t1 VALUES (1, 1);
  SAVEPOINT s1; UPDATE t1 SET c0 = 2; ROLLBACK TO s1;
  CLUSTER t1 USING t1_i0;
  SELECT count(*) FROM t1;   -- 0, expected 1
  COMMIT;
  SELECT count(*) FROM t1;   -- 0, expected 1

Reproduced with default settings on 18.6 (REL_18_STABLE a85b5523e67),
14.24 and master 158b6d5109 (all --enable-cassert --enable-debug, macOS
arm64); also found on master on Ubuntu x86_64. Rechecked on 2026-10-15 (Ubuntu 24.04, x86_64, gcc 13.3, --enable-cassert --enable-debug, server under ulimit -v 6 GB):
the same on 19rc1 (REL_19_STABLE 0fb192f1b02), master aa7b9cf132b and
REL_18_STABLE 4d62ef92fda. CLUSTER VERBOSE reports a
negative "dead row versions cannot be removed yet" count (-1 here).

The same happens with a PL/pgSQL EXCEPTION block instead of the savepoint,
and with REPACK t1 [USING INDEX] on master. It needs CLUSTER's index-scan
path (the index above is created before any rows, so reltuples is -1 and
CLUSTER scans the index); the seqscan+sort path keeps the row, so on a
real table whether rows are lost depends on the plan. Only the rows
touched by the rolled-back update are lost: on an analyzed 10,000-row
table where 100 newly inserted rows were updated in a rolled-back
subtransaction, CLUSTER lost all 100 on 18.6. A HOT update is always
affected; a non-HOT update when the new key sorts after the old one.
Rows committed by an earlier transaction, a rolled-back DELETE or
SELECT FOR UPDATE, and a released subtransaction are not affected.
VACUUM FULL cannot run inside a transaction block and is not affected.

Cause, as far as I can tell: before CLUSTER the row's original version
has xmin = the current transaction, xmax = the aborted subtransaction,
no HEAP_XMAX_INVALID, HOT_UPDATED, ctid -> the aborted new version.
HeapTupleSatisfiesVacuumHorizon() returns HEAPTUPLE_INSERT_IN_PROGRESS
for it in the branch commented "deleting subtransaction must have
aborted", but unlike HeapTupleSatisfiesMVCC() it does not set
HEAP_XMAX_INVALID there. heapam_relation_copy_for_cluster() then passes
the live tuple to rewrite_heap_tuple(), which sees a valid xmax and a
ctid pointing elsewhere and parks it in rs_unresolved_tups as waiting
for its successor. The aborted successor is classified DEAD, and
rewrite_heap_dead_tuple() frees the parked tuple, on the assumption that
a tuple whose successor is dead is dead too. The new heap ends up empty.

Setting the hint bit in that branch fixes all the variants above, and
"make check" passes (239 tests):

--- a/src/backend/access/heap/heapam_visibility.c
+++ b/src/backend/access/heap/heapam_visibility.c
@@ -1177,7 +1177,17 @@ HeapTupleSatisfiesVacuumHorizon(HeapTuple htup, Buffer buffer, TransactionId *de
 			/* inserted and then deleted by same xact */
 			if (TransactionIdIsCurrentTransactionId(HeapTupleHeaderGetUpdateXid(tuple)))
 				return HEAPTUPLE_DELETE_IN_PROGRESS;
-			/* deleting subtransaction must have aborted */
+			/*
+			 * Deleting subtransaction must have aborted.  Mark the xmax
+			 * invalid, like HeapTupleSatisfiesMVCC() does: callers such as
+			 * heapam_relation_copy_for_cluster() rely on HEAP_XMAX_INVALID to
+			 * tell that the tuple is not part of a live update chain, and
+			 * rewrite_heap_tuple() would otherwise wait for the aborted
+			 * newer version and drop this tuple along with it.
+			 */
+			if (!(tuple->t_infomask & HEAP_XMAX_IS_MULTI))
+				SetHintBits(tuple, buffer, HEAP_XMAX_INVALID,
+							InvalidTransactionId);
 			return HEAPTUPLE_INSERT_IN_PROGRESS;
 		}
 		else if (TransactionIdIsInProgress(HeapTupleHeaderGetRawXmin(tuple)))

(A more defensive alternative would be for rewrite_heap_dead_tuple() to
check that the parked tuple's updater actually committed.)

Found with a query fuzzer.

Regards,
<name>
```

## Verification

- minimal.sql gives 0, 0 on 18.6 with default settings (fresh initdb, autovacuum on) and on master 158b6d5109
  (main session, 2026-10-14).
- The fuzzer's original case (model property: DELETE wrote 0 rows, expected 1) on master 49103c3cd63, Ubuntu x86_64:
  workspace-finding/.
- 14.24, the PL/pgSQL variant, REPACK variants, the non-HOT and committed/released/DELETE/FOR UPDATE controls, the
  10,000-row table (100 of 100 lost on 18.6), CLUSTER VERBOSE's negative count, the pageinspect state before CLUSTER,
  and the patched build's make check: triage subagent, triage/matrix.out, triage/pageinspect.out,
  triage/fix-make-check.out.
- REL_19 not built (same code). Duplicate search: two web searches, no match; pgsql-bugs not searched separately.
