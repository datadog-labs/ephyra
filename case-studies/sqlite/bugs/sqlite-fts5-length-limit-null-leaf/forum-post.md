**Title:** FTS5: if a leaf is longer than SQLITE_LIMIT_LENGTH, an INSERT succeeds and stores NULL in its place

This is the FTS5 counterpart of the R-Tree report (https://sqlite.org/bugs/forumpost/38492ae5dc): the same unchecked sqlite3_bind_blob() return code, in fts5DataWrite(). With the length limit set below the size of an FTS5 leaf, an INSERT reports success but the leaf is stored as NULL, and the index stays corrupt after the limit is raised.

```sql
CREATE VIRTUAL TABLE ft USING fts5(a);
.limit length 100
INSERT INTO ft VALUES('a b c d e f g h i j k l m n o p q r s t u v w x y z');
.limit length 1000000000
SELECT id, typeof(block) FROM ft_data;
SELECT rowid FROM ft('a');
INSERT INTO ft(ft) VALUES('integrity-check');
```

Output from trunk (check-in 6b81ddfd0690, 2026-11-03 15:24), built from the source tarball with --enable-fts5:

```
1|blob
10|blob
137438953473|null
Error near line 6: fts5: corruption found reading blob 137438953473 from table "ft"
Error near line 7: fts5: corruption found reading blob 137438953473 from table "ft"
```

Expected: the INSERT fails with "string or blob too big" and the index stays intact, or the INSERT succeeds and stores the full leaf.

If the leaf is longer than the limit, bindText() leaves the parameter NULL and returns SQLITE_TOOBIG, and the cached `REPLACE INTO %_data` statement then stores NULL and succeeds. The averages record and the %_docsize blob are written the same way (for example in a 31-column contentless table at the minimum limit of 30).

The patch below checks these return codes. FTS5 writes leaves when it flushes pending data (at commit, when a statement opens a savepoint, or when the hash table is full), and a failed flush already keeps the transaction from committing (Fts5Index.flushRc), so no further change is needed. The patch also checks the binds of the term lookups in fts5SegIterSeekInit() and fts5SegIterNextInit(): a MATCH for a token one byte shorter than the limit (its index key has a prefix byte) otherwise returns no rows on a multi-leaf segment, without an error. With it, the INSERT above fails with "string or blob too big" and the index stays intact:

```diff
--- a/ext/fts5/fts5_index.c
+++ b/ext/fts5/fts5_index.c
@@ -984,9 +984,11 @@
   }
 
   sqlite3_bind_int64(p->pWriter, 1, iRowid);
-  sqlite3_bind_blob(p->pWriter, 2, pData, nData, SQLITE_STATIC);
-  sqlite3_step(p->pWriter);
-  p->rc = sqlite3_reset(p->pWriter);
+  p->rc = sqlite3_bind_blob(p->pWriter, 2, pData, nData, SQLITE_STATIC);
+  if( p->rc==SQLITE_OK ){
+    sqlite3_step(p->pWriter);
+    p->rc = sqlite3_reset(p->pWriter);
+  }
   sqlite3_bind_null(p->pWriter, 2);
 }
 
@@ -2742,7 +2744,8 @@
   pIdxSelect = fts5IdxSelectStmt(p);
   if( p->rc ) return;
   sqlite3_bind_int(pIdxSelect, 1, pSeg->iSegid);
-  sqlite3_bind_blob(pIdxSelect, 2, pTerm, nTerm, SQLITE_STATIC);
+  p->rc = sqlite3_bind_blob(pIdxSelect, 2, pTerm, nTerm, SQLITE_STATIC);
+  if( p->rc ) return;
   if( SQLITE_ROW==sqlite3_step(pIdxSelect) ){
     i64 val = sqlite3_column_int(pIdxSelect, 0);
     iPg = (int)(val>>1);
@@ -2834,7 +2837,8 @@
   if( pSel ){
     assert( p->rc==SQLITE_OK );
     sqlite3_bind_int(pSel, 1, pSeg->iSegid);
-    sqlite3_bind_blob(pSel, 2, pTerm, nTerm, SQLITE_STATIC);
+    p->rc = sqlite3_bind_blob(pSel, 2, pTerm, nTerm, SQLITE_STATIC);
+    if( p->rc ) return;
 
     if( sqlite3_step(pSel)==SQLITE_ROW ){
       i64 val = sqlite3_column_int64(pSel, 0);
@@ -4334,10 +4338,13 @@
     const char *z = (pWriter->btterm.n>0?(const char*)pWriter->btterm.p:"");
     /* The following was already done in fts5WriteInit(): */
     /* sqlite3_bind_int(p->pIdxWriter, 1, pWriter->iSegid); */
-    sqlite3_bind_blob(p->pIdxWriter, 2, z, pWriter->btterm.n, SQLITE_STATIC);
+    p->rc = sqlite3_bind_blob(p->pIdxWriter, 2, z, pWriter->btterm.n,
+                              SQLITE_STATIC);
     sqlite3_bind_int64(p->pIdxWriter, 3, bFlag + ((i64)pWriter->iBtPage<<1));
-    sqlite3_step(p->pIdxWriter);
-    p->rc = sqlite3_reset(p->pIdxWriter);
+    if( p->rc==SQLITE_OK ){
+      sqlite3_step(p->pIdxWriter);
+      p->rc = sqlite3_reset(p->pIdxWriter);
+    }
     sqlite3_bind_null(p->pIdxWriter, 2);
   }
   pWriter->iBtPage = 0;
--- a/ext/fts5/fts5_storage.c
+++ b/ext/fts5/fts5_storage.c
@@ -764,9 +764,11 @@
       }
     }
     if( rc==SQLITE_OK ){
-      sqlite3_bind_blob(pReplace, 2, pBuf->p, pBuf->n, SQLITE_STATIC);
-      sqlite3_step(pReplace);
-      rc = sqlite3_reset(pReplace);
+      rc = sqlite3_bind_blob(pReplace, 2, pBuf->p, pBuf->n, SQLITE_STATIC);
+      if( rc==SQLITE_OK ){
+        sqlite3_step(pReplace);
+        rc = sqlite3_reset(pReplace);
+      }
       sqlite3_bind_null(pReplace, 2);
     }
   }
```

In an explicit transaction the patch still leaves a partial row if the %_docsize blob of a contentless table is too long: the INSERT now fails, but the rows it wrote before that write are not undone, and after COMMIT a rank query reports "database disk image is malformed".

**Versions:** wrong in 3.54.1 (amalgamation built with -DSQLITE_ENABLE_FTS5), 3.51.0 (/usr/bin/sqlite3 on macOS) and trunk.
