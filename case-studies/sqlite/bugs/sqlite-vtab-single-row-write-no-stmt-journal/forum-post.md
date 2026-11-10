**Title:** FTS5, FTS3/4, R-Tree, Geopoly: a single-row write that fails inside a transaction leaves the module's partial shadow-table writes, and COMMIT keeps them

```c
#include <stdio.h>
#include "sqlite3.h"

/* An FTS5 tokenizer that splits on spaces and rejects non-ASCII text. */
static int xCreate(void *p, const char **az, int n, Fts5Tokenizer **pp){
  *pp = (Fts5Tokenizer*)p;
  return SQLITE_OK;
}
static void xDelete(Fts5Tokenizer *p){}
static int xTokenize(Fts5Tokenizer *p, void *pCtx, int flags,
                     const char *z, int n,
                     int (*xToken)(void*, int, const char*, int, int, int)){
  int i = 0, j, rc = SQLITE_OK;
  while( rc==SQLITE_OK && i<n ){
    while( i<n && z[i]==' ' ) i++;
    for(j=i; j<n && z[j]!=' '; j++){
      if( (unsigned char)z[j]>=0x80 ) return SQLITE_ERROR;
    }
    if( j>i ) rc = xToken(pCtx, 0, z+i, j-i, i, j);
    i = j;
  }
  return rc;
}

static int print_row(void *p, int n, char **az, char **azCol){
  int i;
  for(i=0; i<n; i++) printf("%s%s", i ? "|" : "", az[i]);
  printf("\n");
  return 0;
}
static void run(sqlite3 *db, const char *zSql){
  char *zErr = 0;
  if( sqlite3_exec(db, zSql, print_row, 0, &zErr) ){
    printf("%s: %s\n", zSql, zErr);
    sqlite3_free(zErr);
  }
}

int main(void){
  sqlite3 *db;
  sqlite3_stmt *pStmt;
  fts5_api *pApi = 0;
  fts5_tokenizer tok = { xCreate, xDelete, xTokenize };

  sqlite3_open(":memory:", &db);
  sqlite3_prepare_v2(db, "SELECT fts5(?1)", -1, &pStmt, 0);
  sqlite3_bind_pointer(pStmt, 1, (void*)&pApi, "fts5_api_ptr", 0);
  sqlite3_step(pStmt);
  sqlite3_finalize(pStmt);
  pApi->xCreateTokenizer(pApi, "ascii", 0, &tok, 0);

  run(db, "CREATE VIRTUAL TABLE ft USING fts5(x, tokenize=ascii)");
  run(db, "INSERT INTO ft VALUES('one')");
  run(db, "BEGIN");
  run(db, "INSERT INTO ft VALUES('two caf\xc3\xa9')");
  run(db, "COMMIT");
  run(db, "SELECT rowid, x FROM ft");
  run(db, "SELECT rowid FROM ft('two')");
  run(db, "INSERT INTO ft(ft) VALUES('integrity-check')");
  sqlite3_close(db);
  return 0;
}
```

Output from trunk (check-in 6b81ddfd0690, 2026-11-03 15:24), built from the source tarball with --enable-fts5 --enable-rtree. 3.54.1 and 3.51.0 give the same:

```
INSERT INTO ft VALUES('two café'): SQL logic error
1|one
2|two café
2
INSERT INTO ft(ft) VALUES('integrity-check'): database disk image is malformed
```

Expected: the failed INSERT leaves nothing behind, so the SELECT returns only row 1, the MATCH query nothing and the integrity-check no error. The tokenizer rejects the text, which fts5.h allows ("it may abandon the tokenization and return any error code other than SQLITE_OK or SQLITE_DONE"). Instead row 2 was stored in ft_content and its first token was indexed, but it has no ft_docsize row, and the transaction stays open and COMMIT keeps that. Without BEGIN and COMMIT, or with the INSERT written as `INSERT INTO ft SELECT 'two café'`, the failed INSERT leaves nothing behind.

R-Tree does the same when the error is a shared-cache table lock (`sqlite3 < repro-rtree.sql` in an empty directory):

```sql
.open file:rt.db?cache=shared
CREATE VIRTUAL TABLE rt USING rtree(id, x0, x1);
INSERT INTO rt VALUES(1, 0, 1);
.connection 1
.open file:rt.db?cache=shared
BEGIN;
SELECT count(*) FROM rt_rowid;
.connection 0
BEGIN;
INSERT INTO rt VALUES(2, 5, 6);
COMMIT;
.connection 1
COMMIT;
.connection 0
SELECT * FROM rt;
SELECT rtreecheck('rt');
```

```
1
Error near line 10: database table is locked
1|0.0|1.0
2|5.0|6.0
Mapping (2 -> 1) missing from %_rowid table
Wrong number of entries in %_rowid table - expected 2, actual 1
```

The same script with an FTS5 table and the read lock on ft_docsize ends with "malformed inverted index for FTS5 table main.ft" from PRAGMA integrity_check. With an FTS4 table and the lock on f4_docsize, docid 2 is in f4 but not in f4_docsize, and `SELECT matchinfo(f4, 'l') FROM f4 WHERE f4 MATCH 'two'` fails with "database disk image is malformed".

**Cause, as far as we can tell.** A single-row write to a virtual table runs without a statement journal. usesStmtJournal is set only when the statement is both isMultiWrite and mayAbort, and an INSERT of a single VALUES row, or an UPDATE or DELETE that runs one-pass because xBestIndex set SQLITE_INDEX_SCAN_UNIQUE (for example `WHERE rowid=?`), is not isMultiWrite (delete.c even clears it for a one-pass DELETE). An UPDATE through a full-text query goes through an ephemeral table, has a statement journal and leaves nothing behind. When xUpdate then fails with an error other than SQLITE_NOMEM, SQLITE_IOERR, SQLITE_INTERRUPT or SQLITE_FULL, sqlite3VdbeHalt() asks for a statement rollback, which does nothing because there is no statement transaction, so the rows the module wrote through its own statements stay in the open transaction.

The patch below makes OP_VUpdate roll back the whole transaction in that case if xUpdate changed any rows before it failed (as counted by sqlite3_total_changes()), as sqlite3VdbeHalt() already does for SQLITE_FULL without a statement journal. Errors returned before any row is changed, and SQLITE_CONSTRAINT from a module that sets SQLITE_VTAB_CONSTRAINT_SUPPORT, still leave the transaction open. With it, the COMMIT in both examples fails with "cannot commit - no transaction is active" and the tables keep only row 1:

```diff
--- a/src/vdbe.c
+++ b/src/vdbe.c
@@ -8970,6 +8970,7 @@
   int nArg;
   int i;
   sqlite_int64 rowid = 0;
+  i64 nTotalChange;
   Mem **apArg;
   Mem *pX;
 
@@ -8999,6 +9000,7 @@
       pX++;
     }
     db->vtabOnConflict = pOp->p5;
+    nTotalChange = db->nTotalChange;
     rc = pModule->xUpdate(pVtab, nArg, apArg, &rowid);
     db->vtabOnConflict = vtabOnConflict;
     sqlite3VtabImportErrmsg(p, pVtab);
@@ -9014,6 +9016,15 @@
       }
     }else{
       p->nChange++;
+      if( rc && !p->usesStmtJournal && p->errorAction==OE_Abort
+       && db->nTotalChange!=nTotalChange
+      ){
+        /* xUpdate changed rows (for example in shadow tables) before it
+        ** failed. There is no statement journal, so a statement rollback
+        ** cannot undo them. Roll back the transaction instead, as for
+        ** SQLITE_FULL. */
+        p->errorAction = OE_Rollback;
+      }
     }
     if( rc ) goto abort_due_to_error;
   }
```

As with SQLITE_FULL, the rollback also discards the earlier statements of the transaction. The test is a heuristic: it can miss partial state that is not a table row (FTS5 pending terms in some contentless configurations) and writes made with sqlite3_blob_write(). The other option would be a statement journal for every virtual-table write inside a transaction, but FTS3/4 and FTS5 flush their pending terms in xSavepoint, so a transaction of many single-row inserts would then flush the pending terms after every row.

The SQLITE_LIMIT_LENGTH reports for R-Tree (https://sqlite.org/bugs/forumpost/38492ae5dc) and FTS5 (https://sqlite.org/bugs/forumpost/f712b4e116) are two more ways to reach the same partial write.

**Versions**: the same output in 3.51.0 (the macOS system libsqlite3 and /usr/bin/sqlite3, whose error line reads "Runtime error near line 10: database table is locked (6)"), 3.54.1 and trunk.

We found it with a property-based test generator for SQLite that builds VDBE programs directly.
