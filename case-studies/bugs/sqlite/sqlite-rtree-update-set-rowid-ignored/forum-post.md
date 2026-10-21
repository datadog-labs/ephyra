**Title:** R-Tree: `UPDATE ... SET rowid = N` and `INSERT INTO ...(rowid, ...)` ignore the new rowid

```sql
CREATE VIRTUAL TABLE r USING rtree(id, x1, x2);
INSERT INTO r VALUES (1, 0, 1), (2, 2, 3);

UPDATE r SET rowid = 9 WHERE id = 2;
SELECT changes();
SELECT * FROM r ORDER BY id;

UPDATE r SET rowid = 8, x2 = 7 WHERE id = 1;
SELECT * FROM r ORDER BY id;

INSERT INTO r(rowid, x1, x2) VALUES (5, 4, 5);
SELECT * FROM r ORDER BY id;
```

Output from trunk (check-in 185ffd27064b, 2026-10-20 13:10), built from the source tarball with --enable-rtree. 3.51.0 (the macOS /usr/bin/sqlite3) gives the same, and so do 3.46.0, 3.50.2 and 3.53.2:

```
1
1|0.0|1.0
2|2.0|3.0
1|0.0|7.0
2|2.0|3.0
1|0.0|7.0
2|2.0|3.0
3|4.0|5.0
```

Expected, and what an ordinary table `CREATE TABLE r(id INTEGER PRIMARY KEY, x1 REAL, x2 REAL)` prints for the same statements:

```
1
1|0.0|1.0
9|2.0|3.0
8|0.0|7.0
9|2.0|3.0
5|4.0|5.0
8|0.0|7.0
9|2.0|3.0
```

The first column of an R*Tree is its rowid ("similar to an integer primary key column of a normal SQLite table", rtree.html), and `SELECT rowid, id FROM r` shows the same value twice. But a new rowid given under the name `rowid`, `oid` or `_rowid_` is dropped. The first UPDATE reports one changed row and leaves the row at rowid 2. The second one changes x2 and keeps rowid 1. The INSERT stores the row under a new rowid, 3, and `last_insert_rowid()` returns 3. `UPDATE r SET id = 9 WHERE id = 2` and `INSERT INTO r(id, x1, x2) VALUES (5, 4, 5)` work.

This happens for every UPDATE that assigns the rowid under one of these names and does not assign id: the rowid alone, with coordinate or auxiliary columns, `SET rowid = id + 100` over the whole table, UPDATE OR REPLACE onto a rowid that is in use (nothing is replaced), UPDATE ... FROM, an UPDATE in a trigger, and rtree_i32 tables. A new rowid that is already in use does not raise the constraint error: the UPDATE does nothing, and an `INSERT INTO r(rowid, ...)` of a used rowid adds a row under a new rowid. INSERT OR REPLACE with an existing rowid adds a row instead of replacing one.

The cause is in rtreeUpdate(). rtreeInit() declares the table as `CREATE TABLE x(id INT, x1 REAL, ...)`, and a column of a virtual table is never an alias for the rowid, so for SQLite `rowid` and `id` are two different columns. For an UPDATE, xUpdate gets the old rowid in argv[0], the new rowid in argv[1] (the value assigned to `rowid`, `oid` or `_rowid_`, or else the old rowid) and the value of id in argv[2] (the value assigned to id, or else the current value, which is the old rowid). For an INSERT, argv[1] is the rowid from the column list or NULL, and argv[2] is id or NULL. rtreeUpdate() takes the new rowid only from argv[2], so a value in argv[1] is never used. Geopoly does not have this problem: it has no id column and takes the new rowid from argv[1].

The patch below makes rtreeColumn() return no value for the first column when sqlite3_vtab_nochange() is true, so that rtreeUpdate() can tell an UPDATE that does not assign id from one that does. rtreeUpdate() then takes the new rowid from argv[1] when argv[2] is such a "no change" value (UPDATE) or NULL (INSERT), and from argv[2] otherwise, as before:

```diff
--- a/ext/rtree/rtree.c
+++ b/ext/rtree/rtree.c
@@ -1806,6 +1806,9 @@
   if( NEVER(p==0) ) return SQLITE_OK;
   if( p->iCell>=NCELL(pNode) ) return SQLITE_ABORT;
   if( i==0 ){
+    /* Leave the first column without a value if an UPDATE does not assign
+    ** it. rtreeUpdate() then takes the new rowid from aData[1]. */
+    if( sqlite3_vtab_nochange(ctx) ) return SQLITE_OK;
     sqlite3_result_int64(ctx, nodeGetRowid(pRtree, pNode, p->iCell));
   }else if( i<=pRtree->nDim2 ){
     nodeGetCoord(pRtree, pNode, p->iCell, i-1, &c);
@@ -3192,6 +3195,7 @@
   if( nData>1 ){
     int ii;
     int nn = nData - 4;
+    sqlite3_value *pNewRowid;     /* Value of the new rowid */
 
     if( nn > pRtree->nDim2 ) nn = pRtree->nDim2;
     /* Populate the cell.aCoord[] array. The first coordinate is aData[3].
@@ -3226,10 +3230,23 @@
       }
     }
 
+    /* The new rowid is the value of the first column, aData[2]. For SQLite
+    ** that column is not an alias for the rowid, so a new rowid assigned
+    ** using the name "rowid", "oid" or "_rowid_" is in aData[1]. Use it if
+    ** the first column is not assigned by an UPDATE (aData[2] is then a
+    ** "no change" value, see rtreeColumn()) or is NULL in an INSERT. */
+    pNewRowid = aData[2];
+    if( sqlite3_value_nochange(pNewRowid)
+     || (sqlite3_value_type(aData[0])==SQLITE_NULL
+         && sqlite3_value_type(pNewRowid)==SQLITE_NULL)
+    ){
+      pNewRowid = aData[1];
+    }
+
     /* If a rowid value was supplied, check if it is already present in 
     ** the table. If so, the constraint has failed. */
-    if( sqlite3_value_type(aData[2])!=SQLITE_NULL ){
-      cell.iRowid = sqlite3_value_int64(aData[2]);
+    if( sqlite3_value_type(pNewRowid)!=SQLITE_NULL ){
+      cell.iRowid = sqlite3_value_int64(pNewRowid);
       if( sqlite3_value_type(aData[0])==SQLITE_NULL
        || sqlite3_value_int64(aData[0])!=cell.iRowid
       ){
```

The existing duplicate check, OR REPLACE handling and conversion to an integer then apply to the new rowid whichever name it was given under. Statements that assign id are not affected: if both id and the rowid are assigned, the value of id is used, as before. On an ordinary table the last of the two assignments wins (`SET id = 10, rowid = 9` gives 9 there), but xUpdate cannot see the order. Likewise an INSERT cannot tell an id given as NULL from an omitted one, so `INSERT INTO r(rowid, id, x1, x2) VALUES (5, NULL, 4, 5)` stores rowid 5. Declaring id as INTEGER PRIMARY KEY in the vtab schema would not help: sqlite3_declare_vtab() does not make such a column an alias for the rowid.

**Versions:** wrong in 3.46.0, 3.50.2 and 3.53.2 (amalgamations built with -DSQLITE_ENABLE_RTREE), 3.51.0 (/usr/bin/sqlite3 on macOS) and trunk.

The patch does not touch the code that our two earlier R-Tree/Geopoly reports change (`rowid = <non-integer value>` and the Geopoly UPDATE that changes the rowid without _shape), and it applies with or without those patches.

We found it while triaging another R-Tree/Geopoly report: an UPDATE that changes the rowid of a geopoly row without assigning _shape fails, and the same UPDATE on an rtree table, tried for comparison, reported success but left the row where it was.
