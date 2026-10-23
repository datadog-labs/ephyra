**Title:** "SQL logic error" on the next SAVEPOINT or COMMIT, and the transaction is lost, after ROLLBACK TO undoes a CREATE VIRTUAL TABLE (fts5, fts4)

```sql
CREATE TABLE t(a);
BEGIN;
INSERT INTO t VALUES (1);
SAVEPOINT s0;
CREATE VIRTUAL TABLE ft USING fts5(x);
INSERT INTO ft VALUES ('apple');
ROLLBACK TO s0;
SAVEPOINT s1;
COMMIT;
SELECT count(*) FROM t;
```

Output from trunk (check-in 185ffd27064b, 2026-10-20 13:10), built from the source tarball with --enable-fts5. 3.53.4 and 3.51.0 give the same:

```
Error near line 8: SQL logic error
Error near line 9: SQL logic error
0
```

Expected: no error and a count of 1. ROLLBACK TO s0 has undone the CREATE VIRTUAL TABLE and the INSERT into it, and the rest of the transaction should be unaffected. Instead the next SAVEPOINT fails, as does a second CREATE VIRTUAL TABLE ft (plain INSERTs and UPDATEs on t still work), and COMMIT fails and rolls the whole transaction back, so the row inserted into t before s0 is lost. The same happens with `fts4(x)` in place of `fts5(x)`, and without `SAVEPOINT s1` (COMMIT alone fails). It takes a write to the new table before the ROLLBACK TO: without the INSERT it works, and an R-Tree table in the same place works. It also takes a savepoint inside a transaction: if s0 is the outermost savepoint and started the transaction itself (no BEGIN), it works. A virtual table created before s0 is fine too.

**Cause, as far as we can tell.** `sqlite3VtabCallCreate()` (vtab.c) adds the new virtual table to `db->aVTrans` right after xCreate, but does not do what `sqlite3VtabBegin()` does for a table that joins the transaction later: set `VTable.iSavepoint` and call xSavepoint for the open savepoint. The INSERT then finds the table already in aVTrans, so `sqlite3VtabBegin()` returns early, and `iSavepoint` stays 0. At ROLLBACK TO s0, `sqlite3VtabSavepoint()` calls xRollbackTo only if `pVTab->iSavepoint>iSavepoint`, here 0 > 0, so FTS5 is never told: its pending terms for 'apple' survive while its shadow tables are rolled back (and the schema is reset). The next xSavepoint (from SAVEPOINT s1 or the statement transaction of a later statement) or xSync (at COMMIT) makes FTS5 flush the pending terms to `ft_data`, which no longer exists, and the SQLITE_ERROR comes back without a message. FTS3/4 flush their pending terms in the same places. When s0 started the transaction, ROLLBACK TO passes -1, so 0 > -1 and xRollbackTo is called, which is why that case works (vtab1-24.0 to 24.3 cover that form). It is not specific to FTS: any module with xSavepoint/xRollbackTo misses the xRollbackTo; FTS shows it as an error because it buffers writes, and R-Tree has no buffer.

xCreate already stands in for xBegin for a table created inside a transaction, so only the savepoint level is missing. The patch below does in `sqlite3VtabCallCreate()` what `sqlite3VtabBegin()` does (level `nStatement+nSavepoint`, which includes the CREATE's own statement level) and gives the expected output above:

```diff
--- a/src/vtab.c
+++ b/src/vtab.c
@@ -800,7 +800,17 @@
   if( rc==SQLITE_OK && ALWAYS(sqlite3GetVTable(db, pTab)) ){
     rc = growVTrans(db);
     if( rc==SQLITE_OK ){
-      addToVTrans(db, sqlite3GetVTable(db, pTab));
+      VTable *pVTab = sqlite3GetVTable(db, pTab);
+      const sqlite3_module *pModule = pMod->pModule;
+      int iSvpt = db->nStatement + db->nSavepoint;
+      addToVTrans(db, pVTab);
+      /* As in sqlite3VtabBegin(): open the current savepoint on the new
+      ** table, so that a ROLLBACK TO that undoes the CREATE also invokes
+      ** its xRollbackTo method. */
+      if( iSvpt && pModule->iVersion>=2 && pModule->xSavepoint ){
+        pVTab->iSavepoint = iSvpt;
+        rc = pModule->xSavepoint(pVTab->pVtab, iSvpt-1);
+      }
     }
   }
 
```

One visible side effect: CREATE VIRTUAL TABLE inside an explicit transaction now issues an xSavepoint/xRelease pair for its statement level even with no user savepoint (a no-op for FTS3/4/5 and R-Tree). The `iVersion>=2` check is stricter than the one in `sqlite3VtabBegin()` and matches `sqlite3VtabSavepoint()`.

**Versions**: the same output (with fts5 and with fts4) in every one we tried, 3.46.0, 3.50.2 and 3.53.2 (amalgamations built with -DSQLITE_ENABLE_FTS5), 3.51.0 (the macOS /usr/bin/sqlite3, dated 2025-06-12), 3.53.4 (Homebrew) and trunk.

We found it with a property-based test generator for SQLite databases and queries: a script that created and filled an FTS5 table inside a savepoint, rolled the savepoint back and then opened another one failed with "SQL logic error".
