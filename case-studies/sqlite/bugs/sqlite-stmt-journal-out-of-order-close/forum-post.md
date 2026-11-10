**Title:** Two pending UPDATE ... RETURNING statements finished oldest-first close their statement transactions out of order (since 3.35.0)

An `UPDATE ... RETURNING` makes all of its changes before it returns its first row, but it keeps its statement transaction open until it halts. If two such statements are each stepped once, and the older one is then finished first, their statement transactions are closed out of order:

```c
#include <stdio.h>
#include "sqlite3.h"

int main(void){
  sqlite3 *db;
  sqlite3_stmt *pA, *pB;
  sqlite3_open(":memory:", &db);
  sqlite3_exec(db, "CREATE TABLE t(a INTEGER PRIMARY KEY, b UNIQUE);"
                   "INSERT INTO t VALUES(1,1),(2,2),(3,3);"
                   "BEGIN;", 0, 0, 0);
  sqlite3_prepare_v2(db, "UPDATE t SET b=b+10 RETURNING a", -1, &pA, 0);
  sqlite3_prepare_v2(db, "UPDATE t SET b=b+100 RETURNING a", -1, &pB, 0);
  sqlite3_step(pA);                        /* A pending after its first row */
  sqlite3_step(pB);                        /* B pending after its first row */
  while( sqlite3_step(pA)==SQLITE_ROW ){}  /* finish A first */
  while( sqlite3_step(pB)==SQLITE_ROW ){}
  sqlite3_finalize(pA);
  sqlite3_finalize(pB);
  printf("COMMIT: %d\n", sqlite3_exec(db, "COMMIT", 0, 0, 0));
  sqlite3_close(db);
  return 0;
}
```

With `-DSQLITE_DEBUG`, trunk (check-in 6b81ddfd0690, 2026-11-03 15:24) fails as below, and 3.54.1 the same way (at line 90712):

```
Assertion failed: (p->iStatement==(db->nStatement+db->nSavepoint)), function vdbeCloseStatement, file sqlite3.c, line 90926.
```

In 3.34.0, `OP_ResultRow` released any open statement transaction before returning a row, "to ensure that statement-transactions are always nested, not overlapping". That release is gone since 3.35.0, the release that added RETURNING, so a DML statement now returns rows while it still holds its statement transaction. The patch below puts the release back:

```diff
--- a/src/vdbe.c
+++ b/src/vdbe.c
@@ -1828,6 +1828,17 @@
   }
 #endif
   if( db->mallocFailed ) goto no_mem;
+
+  /* A DML statement returns rows (RETURNING output or a change count)
+  ** only after all of its changes are made and OP_FkCheck has passed.
+  ** Release its statement transaction before returning control to the
+  ** application, which may step another statement that opens a
+  ** statement transaction before this one halts.  Otherwise the two
+  ** statement transactions would overlap instead of nesting. */
+  if( p->iStatement ){
+    rc = sqlite3VdbeCloseStatement(p, SAVEPOINT_RELEASE);
+    if( rc ) goto abort_due_to_error;
+  }
   if( db->mTrace & SQLITE_TRACE_ROW ){
     db->trace.xV2(SQLITE_TRACE_ROW, db->pTraceArg, p, 0);
   }
```

With it, the program above runs without the assertion and the table ends with b = 111, 112 and 113.

A release build gives the same result without the patch, unless the older statement fails after the out-of-order point. Then its statement rollback also undoes the newer statement: if A is changed to `RETURNING a, randomblob(100000*a)` and `sqlite3_hard_heap_limit64(sqlite3_memory_used()+1000)` is called just before A is finished, A fails with SQLITE_NOMEM, B still ends with SQLITE_DONE and the COMMIT succeeds, but b is back to 1, 2 and 3: B's update is lost. With the patch, b is 111, 112 and 113.

The patch also changes one thing for a single statement: an `UPDATE ... RETURNING` inside a transaction that fails after its first row now keeps its changes rather than rolling back its statement transaction, as it already does when it is reset after its first row.
