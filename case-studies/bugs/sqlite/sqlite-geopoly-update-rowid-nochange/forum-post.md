**Title:** Geopoly: an UPDATE that changes the rowid but not _shape fails with "_shape does not contain a valid polygon"

```sql
CREATE VIRTUAL TABLE g USING geopoly(a);
INSERT INTO g(rowid, _shape, a) VALUES
  (1, '[[0,0],[1,0],[0,1],[0,0]]', 'x'),
  (2, '[[5,5],[6,5],[5,6],[5,5]]', 'y');

UPDATE g SET rowid = 7 WHERE rowid = 1;
UPDATE g SET rowid = 8, _shape = _shape WHERE rowid = 2;
SELECT rowid, a FROM g ORDER BY rowid;
```

Output from trunk (check-in 185ffd27064b, 2026-10-20 13:10), built from the source tarball with --enable-geopoly. 3.46.0, 3.50.2 and 3.53.2 give the same:

```
Error near line 6: _shape does not contain a valid polygon
1|x
8|y
```

Expected: the first UPDATE moves row 1 to rowid 7, as the second one (which also sets `_shape = _shape`) moves row 2 to rowid 8, and as an ordinary table or an rtree table does:

```
7|x
8|y
```

The polygon is valid; the same UPDATE with `_shape = _shape` added works, and the row keeps its shape and is found under its new rowid by `geopoly_overlap()`. Every UPDATE that gives a row a different rowid without assigning _shape fails this way: the rowid alone, the rowid and an auxiliary column, `rowid = rowid + 100` over the whole table, UPDATE OR REPLACE and UPDATE OR IGNORE, UPDATE ... FROM and an UPDATE in a trigger. When the new rowid is already in use, the statement fails with this message instead of the constraint error. An UPDATE of auxiliary columns only, or one that sets the rowid to its current value, works. R-Tree has no such problem: `UPDATE r SET id = 7 WHERE id = 1` works, because rtreeColumn() always returns the coordinates and so rtreeUpdate() never sees an "unchanged" value.

The cause is in geopolyUpdate(). For a column that an UPDATE does not assign, SQLite calls xColumn with sqlite3_vtab_nochange() true, and geopolyColumn() returns no value for _shape in that case, so aData[2] is an "unchanged" value (sqlite3_value_nochange() is true) that holds nothing. geopolyUpdate() rebuilds the r-tree entry when the shape changes or when the rowid changes (the `oldRowid!=newRowid` test, commented "Rowid change"), and in both cases it takes the bounding box from aData[2]. For a rowid change with an unchanged _shape there is no polygon there, geopolyBBox() fails, and the error above is returned. The shape is needed a second time later: the old row, which holds _shape in the %_rowid table, is deleted and a new %_rowid row is written for the new rowid, and with an unchanged aData[2] the code binds NULL for _shape there (`a0=coalesce(?2,a0)` then keeps the NULL of the new row). So taking only the bounding box from the old r-tree cell would not be enough; the shape itself has to be carried over. We tried it: a build that did only that made the UPDATE succeed but left `_shape` NULL, so geopoly_overlap() and geopoly_within() no longer find the row, while PRAGMA integrity_check still says ok.

The patch below reads the current _shape of the old row from the %_rowid table when the rowid changes and _shape is unchanged, and uses it in place of aData[2] for the bounding box and for the new %_rowid row:

```diff
--- a/ext/rtree/geopoly.c
+++ b/ext/rtree/geopoly.c
@@ -1621,6 +1621,8 @@
   i64 newRowid;                   /* The new rowid */
   int newRowidValid;              /* True if newRowid is valid */
   int coordChange = 0;            /* Change in coordinates */
+  sqlite3_value *pShape = 0;      /* New _shape value */
+  sqlite3_value *pOldShape = 0;   /* Copy of the unchanged _shape */
 
   if( pRtree->nNodeRef ){
     /* Unable to write to the btree while another cursor is reading from it,
@@ -1636,14 +1638,39 @@
   newRowidValid = nData>1 && sqlite3_value_type(aData[1])!=SQLITE_NULL;
   newRowid = newRowidValid ? sqlite3_value_int64(aData[1]) : 0;
   cell.iRowid = newRowid;
+
+  if( nData>1 ){
+    pShape = aData[2];
+    if( oldRowidValid && oldRowid!=newRowid && sqlite3_value_nochange(pShape) ){
+      /* The rowid changes but _shape does not. aData[2] holds no value in
+      ** this case, so read the current _shape from the %_rowid table. It
+      ** is needed for the bounding box of the new r-tree entry and for the
+      ** %_rowid record of the new rowid. */
+      sqlite3_stmt *pRead = 0;
+      rc = sqlite3_prepare_v3(pRtree->db, pRtree->zReadAuxSql, -1, 0,
+                              &pRead, 0);
+      if( rc==SQLITE_OK ){
+        int rc2;
+        sqlite3_bind_int64(pRead, 1, oldRowid);
+        if( sqlite3_step(pRead)==SQLITE_ROW ){
+          pOldShape = sqlite3_value_dup(sqlite3_column_value(pRead, 2));
+          if( pOldShape==0 ) rc = SQLITE_NOMEM;
+          pShape = pOldShape;
+        }
+        rc2 = sqlite3_finalize(pRead);
+        if( rc==SQLITE_OK ) rc = rc2;
+      }
+      if( rc ) goto geopoly_update_end;
+    }
+  }
 
   if( nData>1                                 /* not a DELETE */
    && (!oldRowidValid                         /* INSERT */
-        || !sqlite3_value_nochange(aData[2])  /* UPDATE _shape */
+        || !sqlite3_value_nochange(pShape)    /* UPDATE _shape */
         || oldRowid!=newRowid)                /* Rowid change */
   ){
-    assert( aData[2]!=0 );
-    geopolyBBox(0, aData[2], cell.aCoord, &rc);
+    assert( pShape!=0 );
+    geopolyBBox(0, pShape, cell.aCoord, &rc);
     if( rc ){
       if( rc==SQLITE_ERROR ){
         pVtab->zErrMsg =
@@ -1709,17 +1736,17 @@
     int nChange = 0;
     sqlite3_bind_int64(pUp, 1, cell.iRowid);
     assert( pRtree->nAux>=1 );
-    if( sqlite3_value_nochange(aData[2]) ){
+    if( sqlite3_value_nochange(pShape) ){
       sqlite3_bind_null(pUp, 2);
     }else{
       GeoPoly *p = 0;
-      if( sqlite3_value_type(aData[2])==SQLITE_TEXT
-       && (p = geopolyFuncParam(0, aData[2], &rc))!=0
+      if( sqlite3_value_type(pShape)==SQLITE_TEXT
+       && (p = geopolyFuncParam(0, pShape, &rc))!=0
        && rc==SQLITE_OK
       ){
         sqlite3_bind_blob(pUp, 2, p->hdr, 4+8*p->nVertex, SQLITE_TRANSIENT);
       }else{
-        sqlite3_bind_value(pUp, 2, aData[2]);
+        sqlite3_bind_value(pUp, 2, pShape);
       }
       sqlite3_free(p);
       nChange = 1;
@@ -1735,6 +1762,7 @@
   }
 
 geopoly_update_end:
+  sqlite3_value_free(pOldShape);
   rtreeRelease(pRtree);
   return rc;
 }
```

The shape is read before anything is deleted (with UPDATE OR REPLACE the row holding the new rowid is deleted first, but that is never the old row), and it is copied with sqlite3_value_dup() and the statement finalized at once, so the value does not point into a page of the %_rowid table that the delete then changes. zReadAuxSql is the `SELECT * FROM %_rowid WHERE rowid=?1` statement that geopolyColumn() already uses for _shape; column 2 is a0, which holds _shape. With the patch, each case above gives the same result as the statement with `_shape = _shape` added gives without it. The extra read costs about 1.3 microseconds per moved row compared with the `_shape = _shape` form (50,000 rows moved with `rowid = rowid + 100000`); a prepared statement kept on the Rtree object would remove that if preferred.

**Versions** (Geopoly builds; the macOS /usr/bin/sqlite3 3.51.0 has no Geopoly): wrong in 3.46.0, 3.50.2 and 3.53.2 (amalgamations built with -DSQLITE_ENABLE_GEOPOLY -DSQLITE_ENABLE_RTREE) and trunk.

The patch does not touch the code that our earlier Geopoly report (`rowid = <non-integer value>`, a change to geopolyFilter() and rtree1.test) changes, and the two patches apply in either order; with both applied, ext/rtree still passes.

We found it while extending a property-based test generator for SQLite databases and queries: the generator started moving rows to new rowids, and on a geopoly table every such UPDATE failed unless it also assigned `_shape = _shape`.
