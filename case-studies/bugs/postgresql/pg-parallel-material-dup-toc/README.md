# EXPLAIN ANALYZE: Assert("toc->toc_entry[i].key != key") in shm_toc_insert (master only, since a241bc4fede)

Found 2026-10-16 by the `views` oracle of ephyra-postgresql (round/views-1,
`explain-analyze-rows`, burn seeds 103307-103316). Not reported.

## What

On an assertion-enabled master, EXPLAIN ANALYZE of a parallel plan whose
Parallel Append children share one SubPlan containing a Materialize node
crashes the backend (default settings, empty tables; `minimal.sql`):

    CREATE TABLE p (a int) PARTITION BY LIST (a);
    CREATE TABLE p1 PARTITION OF p FOR VALUES IN (1);
    CREATE TABLE p2 PARTITION OF p FOR VALUES IN (2);
    CREATE TABLE s (b int);
    EXPLAIN (ANALYZE, COSTS OFF, TIMING OFF, SUMMARY OFF, BUFFERS OFF)
      SELECT * FROM p WHERE a NOT IN (SELECT 1 FROM s, s s2);

    TRAP: failed Assert("toc->toc_entry[i].key != key"), File: "shm_toc.c", Line: 190
    shm_toc_insert <- ExecParallelInitializeDSM <- planstate_tree_walker_impl
    <- planstate_tree_walker_impl <- ExecInitParallelPlan <- ExecGather
    <- ... <- ExplainOnePlan          (backtrace.txt)

The plain query runs (0 rows); only ANALYZE (instrumentation) crashes. The
plan: Gather -> Parallel Append -> two Parallel Seq Scans, each with the
filter `NOT (ANY (a = (SubPlan any_1).col1))`, the SubPlan being
Materialize -> Nested Loop -> (Seq Scan, Materialize -> Seq Scan).

## Cause (as far as read)

a241bc4fede ("Show Material node Maximum Storage for parallel workers",
David Rowley, 2026-10-12) added T_MaterialState to the parallel DSM
estimate/initialize walk (execParallel.c), which inserts a TOC entry keyed by
the node's plan_node_id. A SubPlan referenced from two Append children has
one PlanState (es_subplanstates), and planstate_tree_walker visits each
child's subPlan list, so the same Materialize node is initialized twice and
its key inserted twice. Without assertions the second entry is presumably a
duplicate key that shm_toc_lookup never finds (not checked).

Whether other node types initialized there (Sort, Hash, Agg, Memoize, which
also key on plan_node_id) can hit the same double visit through a shared
SubPlan, on master or older branches, is not established: a Sort variant on
18.6 planned differently.

## Versions checked (2026-10-16)

- master 158b6d5109 (macOS arm64, cassert): crashes (minimal.sql, fuzzer-shrunk.sql).
- master 3c157ad11671 (the workspace's current master, Linux x86_64, cassert): crashes.
- master 73e69062c120 + the known-bug fixes (ef56290fdbd7): crashes.
- REL_18_STABLE a85b5523e67 (18.6): no crash, correct output.
- a241bc4fede is only on master (not in REL_19_STABLE).

Not yet done: a duplicate search (pgsql-hackers thread of a241bc4fede),
a non-assert build, a bisect to confirm a241bc4fede (inferred from the code).
