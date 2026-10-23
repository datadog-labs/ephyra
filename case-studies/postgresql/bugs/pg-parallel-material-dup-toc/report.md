# pgsql-bugs report: parallel executor walkers visit a SubPlan shared by Parallel Append children twice

Status: draft, not sent (2026-10-16). Recommended channel: the bug form (pgsql-bugs), see "Channel" at the end;
alternative: reply.md (reply on the a241bc4fede thread).

## Form fields

- PostgreSQL version: choose `19beta4` (the newest the form lists; the body names master, 19rc1, 18.6, 14.24)
- Operating system: `Ubuntu 24.04 x86_64 (also macOS 26 arm64)`
- Short description: `EXPLAIN ANALYZE of a SubPlan shared by Parallel Append children: duplicate shm_toc keys, 4x counts`

## Body (plain text)

```
Hello,

When the children of a Parallel Append share a SubPlan (a qual on a
partitioned table that contains a non-hashed sublink is copied to every
partition's scan), the parallel executor walkers in execParallel.c visit
that SubPlan's PlanState once per referencing child. There is only one
PlanState (es_subplanstates), but planstate_tree_walker() walks the
subPlan list of every child, so each node of the SubPlan is estimated,
initialized in the DSM, and has its instrumentation reported and
retrieved two or more times.

With EXPLAIN ANALYZE this has two effects.

1. On assert-enabled builds of master and REL_19_STABLE the second
   shm_toc_insert() with the node's plan_node_id fails the assertion
   added by 43222b8e53f ("Assert no duplicate keys in shm_toc_insert()").
   On master it takes only default settings and empty tables, because
   a241bc4fede ("Show Material node Maximum Storage for parallel
   workers") made Material, the usual top node of such a SubPlan, one of
   the node types that put an entry in the TOC:

     CREATE TABLE p (a int) PARTITION BY LIST (a);
     CREATE TABLE p1 PARTITION OF p FOR VALUES IN (1);
     CREATE TABLE p2 PARTITION OF p FOR VALUES IN (2);
     CREATE TABLE s (b int);
     EXPLAIN (ANALYZE, COSTS OFF, TIMING OFF, SUMMARY OFF, BUFFERS OFF)
       SELECT * FROM p WHERE a NOT IN (SELECT 1 FROM s, s s2);

     TRAP: failed Assert("toc->toc_entry[i].key != key"), File: "shm_toc.c", Line: 190

     #6  shm_toc_insert (toc=..., key=4, ...) at shm_toc.c:190
     #7  ExecMaterialInitializeDSM (...) at nodeMaterial.c:440
     #8  ExecParallelInitializeDSM (...) at execParallel.c:596
     #9  planstate_walk_subplans (...) at nodeFuncs.c:4838
     #10 planstate_walk_members (...)
     #11 planstate_tree_walker_impl (...)     <- the Parallel Append
     #12 ExecParallelInitializeDSM (...) at execParallel.c:602
     #13 ExecInitParallelPlan (...)

   The plan is Gather -> Parallel Append -> two Parallel Seq Scans, each
   with "Filter: (NOT (ANY (a = (SubPlan any_1).col1)))", where the
   SubPlan is Materialize -> Nested Loop -> (Seq Scan, Materialize -> Seq
   Scan). The query without ANALYZE runs normally.

   The double visit is older than a241bc4fede. On 19rc1 (and on
   a241bc4fede's parent) the same assertion fails whenever the shared
   SubPlan contains any other node with parallel instrumentation, e.g.
   with the same tables:

     EXPLAIN (ANALYZE, COSTS OFF, TIMING OFF, SUMMARY OFF, BUFFERS OFF)
       SELECT * FROM p WHERE a > ALL (SELECT b FROM s ORDER BY b, b + 1);
     -> TRAP ... in ExecSortInitializeDSM

     ... WHERE a > ALL (SELECT b FROM s GROUP BY b);
     -> TRAP ... in ExecAggInitializeDSM

   I also got it in ExecHashInitializeDSM (a hash join in the sublink,
   enable_mergejoin = off), ExecMemoizeInitializeDSM,
   ExecIncrementalSortInitializeDSM, and ExecSeqScanInstrumentInitDSM
   (EXPLAIN (ANALYZE, IO) of "a > ALL (SELECT b FROM s)").

2. On builds without assertions, and on branches without that
   assertion, nothing crashes (the estimate walk counts the node twice
   too, so the DSM is large enough), but EXPLAIN ANALYZE is wrong:

   - ExecParallelReportInstrumentation() adds each worker's counters
     into its slot twice, and ExecParallelRetrieveInstrumentation()
     adds all worker slots into the node's totals twice. Every node in
     the shared SubPlan shows 2x loops/rows in the "Worker N:" lines
     and 4x in the totals.
   - The node's second *InitializeDSM call allocates a second chunk
     under the same key. Workers attach to the first one (shm_toc_lookup
     returns the first match), while the leader's shared_info points at
     the second, which stays zeroed. So the per-worker Sort/Hash/Agg/
     Memoize details, and on master Material's "Storage", are missing.

   Example, on 18.6 and 14.24 (non-assert behaviour is the same on
   master): 200000 rows in p, 1000 rows in s, parallel_workers = 2 on
   the partitions, parallel_setup_cost = 0, parallel_tuple_cost = 0,
   parallel_leader_participation = off,

     EXPLAIN (ANALYZE, VERBOSE, COSTS OFF, TIMING OFF, SUMMARY OFF, BUFFERS OFF)
       SELECT count(*) FROM p WHERE a < ALL (SELECT b + 5 FROM s GROUP BY b);

     SubPlan 1
       ->  Materialize (actual rows=1000 loops=800000)
             Worker 0:  actual rows=1000 loops=198880
             Worker 1:  actual rows=1000 loops=201120
             ->  HashAggregate (actual rows=1000 loops=800000)
                   Group Key: s.b
                   Worker 0:  actual rows=1000 loops=198880
                   Worker 1:  actual rows=1000 loops=201120
                   ->  Seq Scan on public.s (actual rows=1000 loops=8)
                         Worker 0:  actual rows=1000 loops=2
                         Worker 1:  actual rows=1000 loops=2

   The SubPlan runs 200000 times, not 800000. The same query on an
   unpartitioned copy of p shows loops=200000, workers 99892 + 100108,
   "Batches: 1  Memory Usage: 129kB" for each worker, and loops=2 on
   the Seq Scan.

Versions tested (Ubuntu 24.04, x86_64, gcc 13.3, --enable-debug,
CFLAGS=-O2; dynamic_shared_memory_type = mmap; all builds from
2026-10-16):
  - master a261fc2b722, --enable-cassert: the Material query crashes
    (also on macOS 26 arm64, master 158b6d5109f)
  - master a261fc2b722 without assertions: no crash, wrong numbers as in 2.
  - a241bc4fede, --enable-cassert: the Material query crashes; its
    parent c6b8af429b3 runs it fine but crashes on the GROUP BY and
    ORDER BY queries
  - REL_19_STABLE 1212d5d33b1 (19rc1), --enable-cassert: Material query
    runs; GROUP BY / ORDER BY queries crash
  - REL_18_STABLE dfbc1507cdb (18.6) and REL_14_STABLE 14904052e65
    (14.24), --enable-cassert: no crash (no duplicate-key assertion
    there), wrong numbers as in 2.

Patch for master: make the walkers that set up per-node DSM or
accumulate per-node instrumentation process each plan node once, by
remembering the plan_node_ids they have seen. Estimate and InitializeDSM
stay consistent, so the nnodes cross-check still holds.
ExecParallelInitializeWorker and ExecParallelReInitializeDSM are left
alone, because a second visit there only repeats a lookup or a no-op
for the nodes concerned. With it, all the queries above run, the
counts match the unpartitioned case (Materialize loops=20000 for 20000
rows), the per-worker Storage / Batches lines are back, and make check
(also with debug_parallel_query = regress) and the isolation tests
pass. A regression test for partition_prune.sql is below the patch.

diff --git a/src/backend/executor/execParallel.c b/src/backend/executor/execParallel.c
index 316dd3f67c6..cc6964f8829 100644
--- a/src/backend/executor/execParallel.c
+++ b/src/backend/executor/execParallel.c
@@ -120,6 +120,7 @@ typedef struct ExecParallelEstimateContext
 {
 	ParallelContext *pcxt;
 	int			nnodes;
+	Bitmapset  *seen;			/* plan_node_ids already visited */
 } ExecParallelEstimateContext;
 
 /* Context object for ExecParallelInitializeDSM. */
@@ -128,8 +129,19 @@ typedef struct ExecParallelInitializeDSMContext
 	ParallelContext *pcxt;
 	SharedExecutorInstrumentation *instrumentation;
 	int			nnodes;
+	Bitmapset  *seen;			/* plan_node_ids already visited */
 } ExecParallelInitializeDSMContext;
 
+/*
+ * Context object for ExecParallelRetrieveInstrumentation and
+ * ExecParallelReportInstrumentation.
+ */
+typedef struct ExecParallelInstrumentationContext
+{
+	SharedExecutorInstrumentation *instrumentation;
+	Bitmapset  *seen;			/* plan_node_ids already visited */
+} ExecParallelInstrumentationContext;
+
 /* Helper functions that run in the parallel leader. */
 static char *ExecSerializePlan(Plan *plan, EState *estate);
 static bool ExecParallelEstimate(PlanState *planstate,
@@ -141,11 +153,31 @@ static shm_mq_handle **ExecParallelSetupTupleQueues(ParallelContext *pcxt,
 static bool ExecParallelReInitializeDSM(PlanState *planstate,
 										ParallelContext *pcxt);
 static bool ExecParallelRetrieveInstrumentation(PlanState *planstate,
-												SharedExecutorInstrumentation *instrumentation);
+												ExecParallelInstrumentationContext *c);
+static bool ExecParallelNodeSeen(PlanState *planstate, Bitmapset **seen);
 
 /* Helper function that runs in the parallel worker. */
 static DestReceiver *ExecParallelGetReceiver(dsm_segment *seg, shm_toc *toc);
 
+/*
+ * A SubPlan referenced from more than one place in the plan tree (for
+ * instance, a qual copied to each child of an Append) has a single PlanState,
+ * which planstate_tree_walker() visits once per reference.  Walkers that set
+ * up per-node shared memory or accumulate per-node instrumentation must
+ * process such a subtree only once.  Returns true if this node was already
+ * visited, otherwise remembers it and returns false.
+ */
+static bool
+ExecParallelNodeSeen(PlanState *planstate, Bitmapset **seen)
+{
+	int			plan_node_id = planstate->plan->plan_node_id;
+
+	if (bms_is_member(plan_node_id, *seen))
+		return true;
+	*seen = bms_add_member(*seen, plan_node_id);
+	return false;
+}
+
 /*
  * Create a serialized representation of the plan to be sent to each worker.
  */
@@ -249,6 +281,9 @@ ExecParallelEstimate(PlanState *planstate, ExecParallelEstimateContext *e)
 	if (planstate == NULL)
 		return false;
 
+	if (ExecParallelNodeSeen(planstate, &e->seen))
+		return false;
+
 	/* Count this node. */
 	e->nnodes++;
 
@@ -488,6 +523,9 @@ ExecParallelInitializeDSM(PlanState *planstate,
 	if (planstate == NULL)
 		return false;
 
+	if (ExecParallelNodeSeen(planstate, &d->seen))
+		return false;
+
 	/* If instrumentation is enabled, initialize slot for this node. */
 	if (d->instrumentation != NULL)
 		d->instrumentation->plan_node_id[d->nnodes] =
@@ -763,7 +801,9 @@ ExecInitParallelPlan(PlanState *planstate, EState *estate,
 	 */
 	e.pcxt = pcxt;
 	e.nnodes = 0;
+	e.seen = NULL;
 	ExecParallelEstimate(planstate, &e);
+	bms_free(e.seen);
 
 	/* Estimate space for instrumentation, if required. */
 	if (estate->es_instrument)
@@ -925,11 +965,13 @@ ExecInitParallelPlan(PlanState *planstate, EState *estate,
 	d.pcxt = pcxt;
 	d.instrumentation = instrumentation;
 	d.nnodes = 0;
+	d.seen = NULL;
 
 	/* Install our DSA area while initializing the plan. */
 	estate->es_query_dsa = pei->area;
 	ExecParallelInitializeDSM(planstate, &d);
 	estate->es_query_dsa = NULL;
+	bms_free(d.seen);
 
 	/*
 	 * Make sure that the world hasn't shifted under our feet.  This could
@@ -1100,8 +1142,9 @@ ExecParallelReInitializeDSM(PlanState *planstate,
  */
 static bool
 ExecParallelRetrieveInstrumentation(PlanState *planstate,
-									SharedExecutorInstrumentation *instrumentation)
+									ExecParallelInstrumentationContext *c)
 {
+	SharedExecutorInstrumentation *instrumentation = c->instrumentation;
 	NodeInstrumentation *instrument;
 	int			i;
 	int			n;
@@ -1109,6 +1152,9 @@ ExecParallelRetrieveInstrumentation(PlanState *planstate,
 	int			plan_node_id = planstate->plan->plan_node_id;
 	MemoryContext oldcontext;
 
+	if (ExecParallelNodeSeen(planstate, &c->seen))
+		return false;
+
 	/* Find the instrumentation for this node. */
 	for (i = 0; i < instrumentation->num_plan_nodes; ++i)
 		if (instrumentation->plan_node_id[i] == plan_node_id)
@@ -1182,7 +1228,7 @@ ExecParallelRetrieveInstrumentation(PlanState *planstate,
 	}
 
 	return planstate_tree_walker(planstate, ExecParallelRetrieveInstrumentation,
-								 instrumentation);
+								 c);
 }
 
 /*
@@ -1286,8 +1332,14 @@ ExecParallelCleanup(ParallelExecutorInfo *pei)
 {
 	/* Accumulate instrumentation, if any. */
 	if (pei->instrumentation)
-		ExecParallelRetrieveInstrumentation(pei->planstate,
-											pei->instrumentation);
+	{
+		ExecParallelInstrumentationContext c;
+
+		c.instrumentation = pei->instrumentation;
+		c.seen = NULL;
+		ExecParallelRetrieveInstrumentation(pei->planstate, &c);
+		bms_free(c.seen);
+	}
 
 	/* Accumulate JIT instrumentation, if any. */
 	if (pei->jit_instrumentation)
@@ -1367,12 +1419,16 @@ ExecParallelGetQueryDesc(shm_toc *toc, DestReceiver *receiver,
  */
 static bool
 ExecParallelReportInstrumentation(PlanState *planstate,
-								  SharedExecutorInstrumentation *instrumentation)
+								  ExecParallelInstrumentationContext *c)
 {
+	SharedExecutorInstrumentation *instrumentation = c->instrumentation;
 	int			i;
 	int			plan_node_id = planstate->plan->plan_node_id;
 	NodeInstrumentation *instrument;
 
+	if (ExecParallelNodeSeen(planstate, &c->seen))
+		return false;
+
 	InstrEndLoop(planstate->instrument);
 
 	/*
@@ -1398,7 +1454,7 @@ ExecParallelReportInstrumentation(PlanState *planstate,
 	InstrAggNode(&instrument[ParallelWorkerNumber], planstate->instrument);
 
 	return planstate_tree_walker(planstate, ExecParallelReportInstrumentation,
-								 instrumentation);
+								 c);
 }
 
 /*
@@ -1610,8 +1666,14 @@ ParallelQueryMain(dsm_segment *seg, shm_toc *toc)
 
 	/* Report instrumentation data if any instrumentation options are set. */
 	if (instrumentation != NULL)
-		ExecParallelReportInstrumentation(queryDesc->planstate,
-										  instrumentation);
+	{
+		ExecParallelInstrumentationContext c;
+
+		c.instrumentation = instrumentation;
+		c.seen = NULL;
+		ExecParallelReportInstrumentation(queryDesc->planstate, &c);
+		bms_free(c.seen);
+	}
 
 	/* Report JIT instrumentation data if any */
 	if (queryDesc->estate->es_jit && jit_instrumentation != NULL)

Regression test, in partition_prune.sql after the other
explain_parallel_append() calls (where parallel_setup_cost,
parallel_tuple_cost and min_parallel_table_scan_size are 0):

  -- A SubPlan referenced by several children of a Parallel Append has a
  -- single PlanState; make sure its per-node shared state is set up only once.
  create table ps_shared (a int) partition by list (a);
  create table ps_shared1 partition of ps_shared for values in (1);
  create table ps_shared2 partition of ps_shared for values in (2);
  create table ps_shared_s (b int) with (parallel_workers = 0);
  select explain_parallel_append('select * from ps_shared where a not in (select 1 from ps_shared_s s1, ps_shared_s s2)');
  select explain_parallel_append('select * from ps_shared where a > all (select b from ps_shared_s order by b, b + 1)');
  drop table ps_shared, ps_shared_s;

On an assert-enabled master the first query crashes and the second
fails on master, a241bc4fede^ and 19rc1. With the patch both run. (The
full patch, with expected output, can be sent as an attachment.)

Found with a query fuzzer.

Regards,
<name>
```

## Verification

Raw outputs, commands and builds: VALIDATION.md in this folder. Workspace scripts and SQL: ~/vmat on
workspace-ephyra-fuzz-pg (run.sh, bt.sh, build.sh, minimal.sql, q-*.sql, data.sql, mat-data.sql, regtest*.sql).

| Claim | Evidence |
|---|---|
| minimal.sql traps on master (cassert), TRAP line, backtrace through ExecMaterialInitializeDSM / planstate_walk_subplans | VALIDATION.md, Check 1: master a261fc2b7220 (fresh build2.sh build) and aa7b9cf132b0; gdb via ~/vmat/bt.sh |
| Also crashes on macOS master 158b6d5109f | README.md / backtrace.txt (earlier session); not re-run |
| Plain query (no ANALYZE) runs | minimal.sql's plain EXPLAIN; README.md (earlier session, query itself returns 0 rows) |
| a241bc4fede is the commit that makes the Material query crash; parent fine | Check 3: worktree builds of a241bc4fede and c6b8af429b3 (cassert), minimal.sql run 3x on parent |
| Parent and 19rc1 already crash with Agg / Sort in the SubPlan | Check 3 (parent q-agg: ExecAggInitializeDSM), Check 4b/4c (19 0fb192f1b02d and 1212d5d33b17: q-agg, regtest2) |
| Hash, Memoize, IncrementalSort, SeqScan IO also trap on 19 | Check 4b "More REL_19..." and "Memoize, top-level Sort, IncrementalSort": gdb frame #7 for each |
| The assert comes from 43222b8e53f, on master and REL_19_STABLE, not on REL_18_STABLE | `git merge-base --is-ancestor` (Check 4b) |
| 18.6 and 14.24 do not crash on any of the queries | Check 4a/4c table |
| Non-assert: no crash; Storage lines missing; 4x totals / 2x worker counts | Check 2 (master-noassert, 3 runs, mat-data.sql), Check 4b (18 4d62ef92fda1) and 4c (18.6 dfbc1507cdb5, 14.24 14904052e653), data.sql vs unpartitioned control |
| Mechanism (Report/Retrieve InstrAggNode twice; workers attach to the first chunk, leader keeps the second) | code reading of execParallel.c ExecParallelReportInstrumentation / ExecParallelRetrieveInstrumentation and the *InitializeDSM functions, in line with the numbers above |
| fix.patch fixes all reproducers and gives correct counts and Storage lines | Check 5 (fix build) |
| make check 239/239, isolation 134/134, debug_parallel_query=regress 239/239 | Check 5 |
| New test fails without the patch, passes with it | Check 5, regtest table |
| fix-toc-once.patch fixes only Material; Sort/Agg still trap and counts still 4x | Check 5 (tocfix build) |
| fix.patch applies to origin/master 2323f03a178 | `git apply --check` (Check 5) |
| No duplicate report | Check 6 (thread of a241bc4fede, 4 archive searches, git log) |
| Not established | when the double visit began (not bisected, reasoned only: Parallel Append is v11); index scan / bitmap heap IO instrumentation paths on 19 not tried; workers do get stats for Memoize/Sort/Hash in the shared case on 18 (Agg checked; the others reasoned from the shared code path) |

## Channel

Recommended: the bug form. The underlying problem is not new in a241bc4fede. It is an assertion failure on
19rc1 cassert builds and wrong EXPLAIN ANALYZE output on every supported release (shown on 14.24 and 18.6),
so it needs a back-branch fix, and committers triage those through pgsql-bugs. a241bc4fede only makes it
reachable with default settings on master. After the bug number exists, a short note on the a241bc4fede thread
pointing to it (reply.md, adapted) lets David Rowley see it quickly. If the owner would rather report only the
master regression, reply.md is ready as is, but it should then still mention the older part.
