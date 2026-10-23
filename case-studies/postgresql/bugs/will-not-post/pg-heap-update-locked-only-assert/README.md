# pg-heap-update-locked-only-assert

Status: triaged and validated 2026-10-21. **Duplicate of an existing, unfixed pgsql-hackers report**:
"Assertion with aborted UPDATE in subtransaction", Jasper Smit, 2025-03-26
(https://www.postgresql.org/message-id/CAOG+RQ6ECiiFH7bb7e1M0bkd0whprp=T0u=rvARSaPOGrY-NDQ@mail.gmail.com).
That thread has the same analysis and a proposed patch, and it stopped in April 2025 without a committer reply. The bug is still present on master.
Nothing filed. A bug-form draft (report-draft.md) was blocked from being written by the harness; its text is in the triage handback. It would go out as a follow-up on that thread, if at all.

- Finding: run 20261021-2202-postgresql-fcdfb3f3b11e, worker 2, round/dml-1. The crash record is in [workspace-finding/](workspace-finding/).
- Crash: `TRAP: failed Assert("HEAP_XMAX_IS_LOCKED_ONLY(infomask_lock_old_tuple)"), File: "heapam.c", Line: 3893` in heap_update.
- Minimal repro: [minimal.sql](minimal.sql) has 1 table, 1 row and one transaction (FOR NO KEY UPDATE; UPDATE in a savepoint that is rolled back and released; then an UPDATE through the TOAST path).
- Versions: fails on master a261fc2b7220 and 3cde90ecc5f8, 19rc1, 18.6 and 14.24, so it is not a regression. The assertion dates from bfa2ab56bb8 (9.6, 2016), and the MultiXactIdExpand shortcut it trips on dates from 0ac5ad5134f2 (9.3).
- Non-assert build: the UPDATE succeeds with correct results, and amcheck is clean before and after VACUUM. The only trace is a non-lock-only multi xmax whose updater aborted, on the old tuple (during the TOAST window) and on the new tuple.
- Evidence: [VALIDATION.md](VALIDATION.md). The scripts are in [tools/](tools/) (reducer, run scripts, gdb output); workspace copy in ~/triage-heaplock.
