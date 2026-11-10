**Title:** R-Tree and Geopoly: if a node is longer than SQLITE_LIMIT_LENGTH, an INSERT succeeds and stores NULL in its place

With the length limit set below the size of an R-Tree node (820 bytes for a 1-dimension table), an INSERT into the R-Tree table reports success, but the node is stored as NULL. The table stays corrupt after the limit is raised and in later connections.

```sql
CREATE VIRTUAL TABLE rt USING rtree(id, x0, x1);
INSERT INTO rt VALUES(1, 0, 10);
.limit length 100
INSERT INTO rt VALUES(2, 5, 6);
.limit length 1000000000
SELECT nodeno, typeof(data) FROM rt_node;
SELECT * FROM rt;
PRAGMA integrity_check;
```

`sqlite3 test.db < repro.sql`, then `sqlite3 test.db "SELECT count(*) FROM rt;"`, with trunk (check-in 6b81ddfd0690, 2026-11-03 15:24), built from the source tarball with --enable-rtree:

```
1|null
Error near line 7: database disk image is malformed
Error near line 8: out of memory
Parse error in 2nd command line argument: undersize RTree blobs in "rt_node" (11)
```

Expected: the second INSERT fails with "string or blob too big" and the table keeps its row, or the INSERT succeeds and stores the full node.

The cause is that nodeWrite() in ext/rtree/rtree.c ignores the return code of sqlite3_bind_blob(). If the node is longer than the limit, bindText() leaves the parameter NULL and returns SQLITE_TOOBIG, and the cached `INSERT OR REPLACE INTO %_node` statement then stores NULL and succeeds. The same happens to the Geopoly _shape blob, which can be longer than its JSON text: a 200-vertex polygon is 1207 bytes of JSON and 1604 bytes of blob, so `.limit length 1500` stores a NULL _shape.

The patch below checks these return codes. In an explicit transaction a failed single-row INSERT into a virtual table is not rolled back by a statement journal, so a node write that fails after the %_rowid table was updated still leaves the R-Tree inconsistent ("Wrong number of entries in %_rowid table"). The patch therefore also makes rtreeUpdate() and geopolyUpdate() return SQLITE_TOOBIG before they write anything if a %_node record (the node plus a 4-byte record header) is longer than the limit. With it, the INSERT above fails with "string or blob too big" and the table keeps its row:

```diff
--- a/ext/rtree/geopoly.c
+++ b/ext/rtree/geopoly.c
@@ -1687,6 +1687,8 @@
       goto geopoly_update_end;
     }
     coordChange = 1;
+    rc = rtreeCheckNodeSize(pRtree);
+    if( rc ) goto geopoly_update_end;
 
     /* If a rowid value was supplied, check if it is already present in 
     ** the table. If so, the constraint has failed. */
@@ -1709,6 +1711,9 @@
   ** record to delete from the r-tree table. The following block does
   ** just that.
   */
+  if( rc==SQLITE_OK && nData==1 ){
+    rc = rtreeCheckNodeSize(pRtree);
+  }
   if( rc==SQLITE_OK && (nData==1 || (coordChange && oldRowidValid)) ){
     rc = rtreeDeleteRowid(pRtree, oldRowid);
   }
@@ -1752,7 +1757,8 @@
        && (p = geopolyFuncParam(0, pShape, &rc))!=0
        && rc==SQLITE_OK
       ){
-        sqlite3_bind_blob(pUp, 2, p->hdr, 4+8*p->nVertex, SQLITE_TRANSIENT);
+        rc = sqlite3_bind_blob(pUp, 2, p->hdr, 4+8*p->nVertex,
+                               SQLITE_TRANSIENT);
       }else{
         sqlite3_bind_value(pUp, 2, pShape);
       }
@@ -1763,7 +1769,7 @@
       nChange++;
       sqlite3_bind_value(pUp, jj+2, aData[jj+2]);
     }
-    if( nChange ){
+    if( rc==SQLITE_OK && nChange ){
       sqlite3_step(pUp);
       rc = sqlite3_reset(pUp);
     }
--- a/ext/rtree/rtree.c
+++ b/ext/rtree/rtree.c
@@ -900,6 +900,22 @@
 }
 
 /*
+** Return SQLITE_TOOBIG if a %_node record is larger than the
+** SQLITE_LIMIT_LENGTH limit, so that nodeWrite() cannot write it. The
+** record is the node blob plus a 4-byte header (header size, a NULL for
+** the INTEGER PRIMARY KEY and a 2-byte serial type, as nodes are between
+** 58 and 8185 bytes). The xUpdate methods check this before they write
+** anything, so that the write does not fail when the shadow tables are
+** already partly updated.
+*/
+static int rtreeCheckNodeSize(Rtree *pRtree){
+  if( pRtree->iNodeSize+4>sqlite3_limit(pRtree->db, SQLITE_LIMIT_LENGTH, -1) ){
+    return SQLITE_TOOBIG;
+  }
+  return SQLITE_OK;
+}
+
+/*
 ** If the node is dirty, write it out to the database.
 */
 static int nodeWrite(Rtree *pRtree, RtreeNode *pNode){
@@ -911,10 +927,13 @@
     }else{
       sqlite3_bind_null(p, 1);
     }
-    sqlite3_bind_blob(p, 2, pNode->zData, pRtree->iNodeSize, SQLITE_STATIC);
-    sqlite3_step(p);
+    rc = sqlite3_bind_blob(p, 2, pNode->zData, pRtree->iNodeSize,
+                           SQLITE_STATIC);
+    if( rc==SQLITE_OK ){
+      sqlite3_step(p);
+      rc = sqlite3_reset(p);
+    }
     pNode->isDirty = 0;
-    rc = sqlite3_reset(p);
     sqlite3_bind_null(p, 2);
     if( pNode->iNode==0 && rc==SQLITE_OK ){
       pNode->iNode = sqlite3_last_insert_rowid(pRtree->db);
@@ -3187,6 +3206,8 @@
     ** cursor. */
     return SQLITE_LOCKED_VTAB;
   }
+  rc = rtreeCheckNodeSize(pRtree);
+  if( rc ) return rc;
   rtreeReference(pRtree);
   assert(nData>=1);
```

In an explicit transaction the patch still leaves a partial row if a Geopoly _shape blob is too long: the INSERT now fails, but the cell it wrote before the _shape write is not undone.

Separately, `PRAGMA integrity_check` reports "out of memory" for the corrupt table, not a corruption message: rtreeCheckGetNode() calls sqlite3_malloc64(0) for the zero-length node and treats the NULL result as an OOM.

**Versions:** wrong in 3.54.1 (amalgamation built with -DSQLITE_ENABLE_RTREE), 3.51.0 (/usr/bin/sqlite3 on macOS) and trunk.

FTS5 has the same problem in fts5DataWrite(); a separate report follows.
