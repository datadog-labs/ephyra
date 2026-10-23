**Title:** UPDATE changes REAL -9223372036854775808.0 in a column it does not assign to INTEGER, but not in the index

```sql
CREATE TABLE t1(a INT, b);
CREATE INDEX t1a ON t1(a);
INSERT INTO t1 VALUES('-9223372036854775809', 1);
UPDATE t1 SET b = 2;
SELECT a FROM t1;
SELECT a, b FROM t1;
PRAGMA integrity_check;
```

Output from trunk (check-in 03f699544e6b, 2026-10-21 23:49):

```
-9.2233720368547758e+18
-9223372036854775808|2
ok
```

Expected, as from check-in 3769f60be38b (2026-10-21 16:05) and 3.54.0:

```
-9.2233720368547758e+18
-9.2233720368547758e+18|2
ok
```

The text rounds to the REAL -9223372036854775808.0, which the INSERT stores as a REAL. The UPDATE does not assign `a`, but on trunk it rewrites the table record with `a` as the INTEGER -9223372036854775808, while the entry in t1a keeps the REAL. The first query reads `a` from the covering index and the second from the table, so the same row shows two different values. `PRAGMA integrity_check` does not notice, because the two compare equal. The same happens with a database written by 3.54.0 or earlier that holds `-9223372036854775808.0` in an INTEGER column (`INSERT INTO t1 VALUES(-9223372036854775808.0, 1)` stores a REAL there): the first UPDATE of another column on trunk makes the table and its indexes disagree. A WITHOUT ROWID table behaves the same way for an index that does not contain the updated column.

The cause is the check-in after 3769f60be38b that added the `bFromText` argument to `sqlite3VdbeIntegerAffinity()`. INTEGER and NUMERIC affinity now turn the REAL -9223372036854775808.0 into an INTEGER unless the value came from text. Applying the affinity is therefore no longer idempotent for a REAL -9223372036854775808.0 that is already stored, whether it came from text or from an earlier version. An UPDATE reads the columns it does not assign from the old row and applies the table affinity to every column of the new row (`sqlite3TableAffinity()` in `sqlite3GenerateConstraintChecks()`), so the table record gets the INTEGER. The indexes are rewritten only if they contain an assigned column, so the others keep the REAL.

The patch below makes an UPDATE apply column affinity only to the columns it assigns and to generated columns. Unassigned REAL columns still get REAL affinity, so that integral values keep their compact integer encoding. Converting text that rounds to -9223372036854775808.0 to an INTEGER as well would cover the first example but not databases written by earlier versions.

```diff
--- a/src/insert.c
+++ b/src/insert.c
@@ -219,8 +219,58 @@
       assert( sqlite3VdbeGetLastOp(v)->opcode==OP_MakeRecord
               || sqlite3VdbeDb(v)->mallocFailed );
       sqlite3VdbeChangeP4(v, -1, zColAff, i);
+    }
+  }
+}
+
+/*
+** Code the affinity transformations for the new row of an UPDATE (aiChng
+** is not NULL) the way sqlite3TableAffinity() does for an INSERT, except
+** that a column the UPDATE does not change (aiChng[i]<0) gets no affinity,
+** or REAL affinity for a REAL column so that integral values are still
+** stored as integers.  The value of an unchanged column was read from the
+** old row.  Applying the column affinity to it again can change how it is
+** stored if the old row was written by a different version of SQLite (for
+** example REAL -9223372036854775808.0 in an INTEGER column), and the new
+** table record would then disagree with the entries of indexes that the
+** UPDATE does not rewrite.  For an INSERT, or for a STRICT table, this
+** routine is the same as sqlite3TableAffinity().
+*/
+static void tableAffinityForChng(Vdbe *v, Table *pTab, int iReg, int *aiChng){
+  sqlite3 *db = sqlite3VdbeDb(v);
+  char *zAff;
+  int i, j;
+  if( aiChng==0 || (pTab->tabFlags & TF_Strict)!=0 ){
+    sqlite3TableAffinity(v, pTab, iReg);
+    return;
+  }
+  zAff = (char*)sqlite3DbMallocRaw(db, pTab->nCol+1);
+  if( zAff==0 ) return;
+  for(i=j=0; i<pTab->nCol; i++){
+    const Column *pCol = &pTab->aCol[i];
+    if( pCol->colFlags & COLFLAG_VIRTUAL ) continue;
+    if( aiChng[i]<0
+     && (pCol->colFlags & COLFLAG_GENERATED)==0
+     && pCol->affinity!=SQLITE_AFF_REAL
+    ){
+      zAff[j++] = SQLITE_AFF_NONE;
+    }else{
+      zAff[j++] = pCol->affinity;
     }
   }
+  do{
+    zAff[j--] = 0;
+  }while( j>=0 && zAff[j]<=SQLITE_AFF_BLOB );
+  j++;
+  if( j==0 ){
+    sqlite3DbFree(db, zAff);
+  }else if( iReg ){
+    sqlite3VdbeAddOp4(v, OP_Affinity, iReg, j, 0, zAff, P4_DYNAMIC);
+  }else{
+    assert( sqlite3VdbeGetLastOp(v)->opcode==OP_MakeRecord
+            || db->mallocFailed );
+    sqlite3VdbeChangeP4(v, -1, zAff, P4_DYNAMIC);
+  }
 }
 
 /*
@@ -2073,7 +2123,7 @@
         continue;
       }
       if( bAffinityDone==0 ){
-        sqlite3TableAffinity(v, pTab, regNewData+1);
+        tableAffinityForChng(v, pTab, regNewData+1, aiChng);
         bAffinityDone = 1;
       }
       allOk = sqlite3VdbeMakeLabel(pParse);
@@ -2422,7 +2472,7 @@
     }
     addrUniqueOk = sqlite3VdbeMakeLabel(pParse);
     if( bAffinityDone==0 ){
-      sqlite3TableAffinity(v, pTab, regNewData+1);
+      tableAffinityForChng(v, pTab, regNewData+1, aiChng);
       bAffinityDone = 1;
     }
     VdbeNoopComment((v, "prep index %s", pIdx->zName));
@@ -2730,7 +2780,7 @@
     sqlite3VdbeAddOp3(v, OP_MakeRecord, regNewData+1, pTab->nNVCol, regRec);
     sqlite3SetMakeRecordP5(v, pTab);
     if( !bAffinityDone ){
-      sqlite3TableAffinity(v, pTab, 0);
+      tableAffinityForChng(v, pTab, 0, aiChng);
     }
   }
 
```

**Versions:** trunk 03f699544e6b. Check-in 3769f60be38b, 3.54.0 and 3.51.0 give the expected output.

We found it with a property-based test generator for SQLite databases and queries: a check compared a query against the same query with INDEXED BY another index.
