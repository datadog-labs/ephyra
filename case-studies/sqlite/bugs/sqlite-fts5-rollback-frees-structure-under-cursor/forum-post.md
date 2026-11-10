**Title:** FTS5: ROLLBACK frees the index structure under an open MATCH query in another statement (heap use-after-free)

```c
#include <stdio.h>
#include "sqlite3.h"
static void ex(sqlite3 *db, const char *sql){
  char *err = 0;
  int rc = sqlite3_exec(db, sql, 0, 0, &err);
  printf("exec %-40s rc=%d %s\n", sql, rc, err ? err : "");
  sqlite3_free(err);
}
int main(void){
  sqlite3 *db; sqlite3_stmt *q; int rc;
  printf("SQLite %s\n", sqlite3_sourceid());
  sqlite3_open(":memory:", &db);
  ex(db, "CREATE VIRTUAL TABLE ft USING fts5(a)");
  ex(db, "INSERT INTO ft VALUES('x'); INSERT INTO ft VALUES('x')");
  ex(db, "BEGIN");
  ex(db, "INSERT INTO ft VALUES('y')");
  sqlite3_prepare_v2(db, "SELECT rowid FROM ft WHERE ft MATCH 'x'", -1, &q, 0);
  rc = sqlite3_step(q);
  printf("step 1 rc=%d rowid=%lld\n", rc, sqlite3_column_int64(q, 0));
  ex(db, "ROLLBACK");
  rc = sqlite3_step(q);
  printf("step 2 rc=%d %s\n", rc, sqlite3_errmsg(db));
  sqlite3_finalize(q);
  sqlite3_close(db);
  return 0;
}
```

Output from trunk (check-in 8a675a4b2b, 2026-11-05 00:01), amalgamation built with -DSQLITE_ENABLE_FTS5 -fsanitize=address -g:

```
step 1 rc=100 rowid=1
exec ROLLBACK                                 rc=0 
==ERROR: AddressSanitizer: heap-use-after-free
READ of size 4 thread T0
    #0 fts5SegIterNextPage sqlite3.c:255556
    #1 fts5SegIterNext sqlite3.c:256079
    #2 fts5MultiIterNext sqlite3.c:257133
    #3 sqlite3Fts5IterNext sqlite3.c:261268
    #4 fts5ExprNodeNext_TERM sqlite3.c:251088
    #5 sqlite3Fts5ExprNext sqlite3.c:251416
    #6 fts5NextMethod sqlite3.c:264344
    #7 sqlite3VdbeExec sqlite3.c:105432
    #8 sqlite3Step sqlite3.c:94430
    #9 sqlite3_step sqlite3.c:94491
    #10 main repro.c:21
freed by thread T0 here:
    #1 sqlite3MemFree sqlite3.c:28495
    #2 sqlite3_free sqlite3.c:32295
    #3 fts5StructureRelease sqlite3.c:254750
    #4 fts5StructureInvalidate sqlite3.c:255087
    #5 sqlite3Fts5IndexRollback sqlite3.c:260585
    #6 sqlite3Fts5StorageRollback sqlite3.c:268765
    #7 fts5RollbackMethod sqlite3.c:265488
    #8 callFinaliser sqlite3.c:164434
    #9 sqlite3VtabRollback sqlite3.c:164474
    #10 sqlite3RollbackAll sqlite3.c:191092
    #11 sqlite3VdbeExec sqlite3.c:100724
    #12 sqlite3Step sqlite3.c:94430
    #13 sqlite3_step sqlite3.c:94491
    #14 sqlite3_exec sqlite3.c:143632
    #15 ex repro.c:5
    #16 main repro.c:20
previously allocated by thread T0 here:
    #7 fts5StructureDecode sqlite3.c:254879
    #8 fts5StructureReadUncached sqlite3.c:254995
    #9 fts5StructureRead sqlite3.c:255051
    #10 sqlite3Fts5IndexLoadConfig sqlite3.c:261577
    #11 fts5FilterMethod sqlite3.c:264918
    #12 sqlite3VdbeExec sqlite3.c:105328
    #13 sqlite3Step sqlite3.c:94430
```

Without ASan (-O2), the second sqlite3_step() returns SQLITE_CORRUPT on a valid database:

```
step 2 rc=11 database disk image is malformed
```

Expected: the second sqlite3_step() returns the second matching row (rc=100), or an error such as SQLITE_ABORT. The same happens when the rollback comes from `INSERT OR ROLLBACK` on another table (repro2.c), and from a failed COMMIT. It takes a write to the FTS5 table earlier in the transaction (so that its xRollback is called) and a MATCH query started after that write. FTS4 gives the expected output with the same program.

**Cause, as far as we can tell.** sqlite3Fts5IndexQuery() releases its reference to the Fts5Structure after building the iterator, but the segment iterators keep pointers into the structure's aSeg arrays, so only the index's cached copy keeps it alive. Before anything frees that cached copy, FTS5 calls fts5TripCursors() to put active MATCH cursors into the REQUIRE_RESEEK state: in xUpdate, in sqlite3Fts5FlushToDisk() (xSavepoint, xRelease) and in xRollbackTo. fts5RollbackMethod() does not. sqlite3Fts5IndexRollback() calls fts5StructureInvalidate(), which frees the cached structure, and the next xNext of the open cursor reads the freed aSeg (pSeg->pgnoLast in fts5SegIterNextPage()). sqlite3RollbackAll() trips b-tree cursors only, so the statement is not aborted either.

The patch below trips the cursors in xRollback too, as xRollbackTo does, and gives the expected output above (step 2 rc=100) for both programs:

```diff
--- a/ext/fts5/fts5_main.c
+++ b/ext/fts5/fts5_main.c
@@ -2155,6 +2155,7 @@
   int rc;
   Fts5FullTable *pTab = (Fts5FullTable*)pVtab;
   fts5CheckTransactionState(pTab, FTS5_ROLLBACK, 0);
+  fts5TripCursors(pTab);
   rc = sqlite3Fts5StorageRollback(pTab->pStorage);
   pTab->p.pConfig->pgsz = 0;
   return rc;
```

**Versions**: the use-after-free in trunk (8a675a4b2b and 6b81ddfd0690) and 3.50.4 (ASan builds of the amalgamation). The macOS system library (3.51.0) returns the second row in this program, so the release-build symptom depends on the heap state.

We found it with a property-based test generator for SQLite that builds VDBE programs directly and keeps some statements suspended while others run on the same connection.
