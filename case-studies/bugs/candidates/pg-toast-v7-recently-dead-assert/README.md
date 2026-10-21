# TRAP Assert("recently_dead") at heapam_handler.c:993: a bug in the v7 patch for BUG #19519

**Verdict: (a), a bug in v7 itself.** It reproduces on master 3c157ad11671 + v7 alone, the same as on the fixes build. The CLUSTER fix doesn't matter. Plain master raises the known #19519 `missing chunk` error at the same point. No live tuple loses TOAST, and no data is lost. In a build without asserts, v7 does the right thing; only the Assert is wrong.

## What

The fuzz run 20261016-1529 on the fixes build (78eb85b903cc) crashed 4 times during `VACUUM (FULL ...) t0/t1`:

    TRAP: failed Assert("recently_dead"), File: "heapam_handler.c", Line: 993

## Cause

The crash is in v7's `heapam_relation_copy_for_cluster()`, on the path without a tuplesort (VACUUM FULL, or CLUSTER without a sort):

    if (!reform_and_rewrite_tuple(..., recently_dead))
    {
        Assert(recently_dead);            /* fails */
        *tups_vacuumed += 1; *num_tuples -= 1; *tups_recently_dead -= 1;
        continue;
    }

The false result comes from `rewrite_heap_tuple()` (rewriteheap.c). Its loop first inserts the caller's tuple. If that tuple is the newer half (B) of an update pair, and the older half (A) was parked earlier in `rs_unresolved_tups`, the loop then switches to A (`recently_dead = unresolved->recently_dead`, which is true) and calls `raw_heap_insert()` again. A's TOAST is gone, so `raw_heap_insert()` with TOAST_MISSING_OK returns false, and `rewrite_heap_tuple()` returns false for the whole call. The caller's own tuple B, which is LIVE (recently_dead = false), has already been written.

The trigger needs the #19519 condition (a RECENTLY_DEAD tuple whose TOAST chunks are already removed, because a transaction in another database holds back VACUUM FULL's OldestXmin). On top of that, the tuple must be the old version from an UPDATE whose new version is LIVE, and the scan must see the old version first. A HOT update does that, because the new version goes later on the same page. In the fuzz script this was `UPDATE t0 SET c1 = NULL WHERE c1 <> ''`, with 13 other workers in other databases holding xids.

A DELETE doesn't trigger it, because there is no successor. Neither does an UPDATE whose new version is scanned before the old one (E7 in VALIDATION.md).

## Repro (minimal.sql)

Run it with `psql -X -d postgres -f minimal.sql`, with psql on PATH and PGHOST and PGPORT set. It has one row: `UPDATE t SET b = NULL`, then `VACUUM` of the TOAST table, then `VACUUM FULL t`, while a background psql in another database holds an xid.

| build | result |
|---|---|
| fixes 78eb85b903cc (master 3c157ad11671 + 10 patches) | TRAP (also repro.sql, 5/5) |
| master 3c157ad11671 + v7 (cfbot cf/6824 1efb423981e) | TRAP (repro.sql 3/3) |
| master + v7 + fix-htsv-xmax-invalid.patch | TRAP |
| master 3c157ad11671 (~/pg-builds/current) | `ERROR: missing chunk number 0 for toast value N` (known #19519) |
| master + v7 + scripts/v7-assert-fix.patch | OK; VERBOSE `found 10 removable, 20 nonremovable`; verify_heapam clean |

All builds are cassert, on Linux x86_64 (workspace-ephyra-fuzz-pg).

scripts/repro.sql is the 20-row variant of #19519's on-access-pruning repro. It removes the TOAST through `VACUUM u` plus a SELECT, not through a direct VACUUM of the TOAST table.

## Master's side

On master, the missing chunks belong only to the old versions (verify_heapam: offsets 2..20 of the 20-row repro). The new versions have b = NULL. VACUUM FULL fails with XX001 and leaves the table intact, with 20 rows. This is the existing #19519 bug, not new data loss. So it isn't (c).

## Fix suggestion

Remove the Assert, and document that the dropped recently-dead tuple may be the older chain member (scripts/v7-assert-fix.patch). The caller's counter updates are already right for that case: the parked A was counted when it was scanned. Alternatively, `rewrite_heap_tuple()` could return false only for the caller's own tuple (scripts/v7-chain-fix-alt.patch), but then the dropped A stays counted as recently dead.

## Files

- VALIDATION.md: experiments E0-E8, with commands and raw results.
- minimal.sql: one-row repro.
- scripts/: cl.sh (throwaway cluster), run.sh, build.sh (worktree + patches + cassert build), repro.sql, repro-amcheck.sql, r-delete.sql, r-noxid.sql, r-border.sql and r-border-control.sql (scan-order ablation), r-verbose.sql, v7-assert-fix.patch, v7-chain-fix-alt.patch.
- workspace-crashes/: the crash records from the run.
- Not filed upstream (per ephyra policy). A reply draft for Matthias is below.

## Draft reply to Matthias (not sent)

    Hi Matthias,

    While testing v7 (cfbot cf/6824, 1efb423981e, on master 3c157ad11671,
    cassert build), I hit the new assertion in
    heapam_relation_copy_for_cluster():

      TRAP: failed Assert("recently_dead"), File: "heapam_handler.c", Line: 993

    This happens when the tuple whose TOAST is gone is the old version
    from an UPDATE, and VACUUM FULL scans it before its live successor. In
    that case rewrite_heap_tuple() parks the old version in
    rs_unresolved_tups. When the live new version arrives, rewrite_heap_tuple()
    inserts it, then loops to insert the parked old version with that
    one's recently_dead = true. raw_heap_insert() returns false for the
    old version, and rewrite_heap_tuple() returns that false to the caller,
    whose own tuple is LIVE and has already been written. So the caller's
    recently_dead is false, and the Assert fires.

    Reproducer, a single row, with psql on PATH:

      CREATE DATABASE toast_other;
      CREATE DATABASE toast_repro;
      \c toast_repro
      CREATE TABLE t (id int, b text STORAGE EXTERNAL);
      INSERT INTO t VALUES (1, repeat('x', 10000));
      SELECT reltoastrelid::regclass AS toast FROM pg_class
        WHERE oid = 't'::regclass \gset
      \! psql -X -d toast_other -Atc "SELECT pg_current_xact_id(), pg_sleep(4)" >/dev/null &
      SELECT pg_sleep(1);
      UPDATE t SET b = NULL;
      VACUUM :toast;
      VACUUM FULL t;

    Unpatched master raises the usual "missing chunk number 0 for toast
    value N" here. With v7, it crashes with the TRAP above. A DELETE
    instead of the UPDATE works fine with v7. So does an UPDATE whose new
    version lands on an earlier page than the old one, because then the
    old version comes in as the caller's own tuple.

    Without asserts the outcome is already correct. The live tuple is in
    the new heap, and the old version is dropped. The counter adjustments
    in the caller also match, because the parked tuple was counted in
    num_tuples and tups_recently_dead when it was scanned. With the
    Assert removed, VACUUM (FULL, VERBOSE) on a 20-row version reports
    "found 10 removable, 20 nonremovable row versions", and
    verify_heapam() is clean afterwards.

    It might be worth adding an UPDATE permutation to
    rewrite_stale_xmin.spec, next to the DELETE ones.

    Regards,
    Alp Keles
