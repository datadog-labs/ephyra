**Title:** RENAME COLUMN fails on an INTEGER PRIMARY KEY column named in a UNIQUE table constraint

```sql
CREATE TABLE t1(c INTEGER PRIMARY KEY, UNIQUE(c)) WITHOUT ROWID;
ALTER TABLE t1 RENAME COLUMN c TO z;
CREATE TABLE t2(c INTEGER, UNIQUE(c), PRIMARY KEY(c));
ALTER TABLE t2 RENAME COLUMN c TO z;
SELECT sql FROM sqlite_schema WHERE type='table';
```

Output from trunk d6f1df8c0bbc (2026-10-23):

```
Error near line 2: error in table t1 after rename: no such column: c
Error near line 4: error in table t2 after rename: no such column: c
CREATE TABLE t1(c INTEGER PRIMARY KEY, UNIQUE(c)) WITHOUT ROWID
CREATE TABLE t2(c INTEGER, UNIQUE(c), PRIMARY KEY(c))
```

Expected:

```
CREATE TABLE t1(z INTEGER PRIMARY KEY, UNIQUE(z)) WITHOUT ROWID
CREATE TABLE t2(z INTEGER, UNIQUE(z), PRIMARY KEY(z))
```

The UNIQUE columns are resolved while the CREATE TABLE is parsed, so a reference to the INTEGER PRIMARY KEY gets `iColumn==-1` if the PRIMARY KEY came first and the column index otherwise. `renameColumnFunc()` in src/alter.c matches only one value computed from the finished table (where a WITHOUT ROWID table has `iPKey==-1`), so the UNIQUE reference is missed in both tables above. With the constraints in the other order both renames succeed.

The patch below makes `renameColumnFunc()` accept both values while it walks the index expressions:

```diff
--- a/src/alter.c
+++ b/src/alter.c
@@ -743,6 +743,7 @@
   RenameToken *pList;             /* List of tokens to overwrite */
   int nList;                      /* Number of tokens in pList */
   int iCol;                       /* Index of column being renamed */
+  int iAltCol;                    /* Another iColumn value for the column */
   Table *pTab;                    /* Table being ALTERed */
   const char *zOld;               /* Old column name */
 };
@@ -1049,7 +1050,7 @@
   ){
     renameTokenFind(pWalker->pParse, p, (void*)pExpr);
   }else if( pExpr->op==TK_COLUMN
-   && pExpr->iColumn==p->iCol
+   && (pExpr->iColumn==p->iCol || pExpr->iColumn==p->iAltCol)
    && ALWAYS(ExprUseYTab(pExpr))
    && p->pTab==pExpr->y.pTab
   ){
@@ -1595,6 +1596,7 @@
   zOld = pTab->aCol[iCol].zCnName;
   memset(&sCtx, 0, sizeof(sCtx));
   sCtx.iCol = ((iCol==pTab->iPKey) ? -1 : iCol);
+  sCtx.iAltCol = sCtx.iCol;
 
 #ifndef SQLITE_OMIT_AUTHORIZATION
   db->xAuth = 0;
@@ -1636,12 +1638,29 @@
           renameTokenFind(&sParse, &sCtx, (void*)&sParse.pNewTable->iPKey);
         }
         sqlite3WalkExprList(&sWalker, sParse.pNewTable->pCheck);
+
+        /* The columns of a PRIMARY KEY or UNIQUE constraint are resolved
+        ** while the CREATE TABLE is still being parsed. A reference to an
+        ** INTEGER PRIMARY KEY column resolves to iColumn==-1 if it follows
+        ** the PRIMARY KEY clause, or to the column index if it precedes it.
+        ** For a WITHOUT ROWID table, the INTEGER PRIMARY KEY is converted
+        ** to a PRIMARY KEY index at the end of the parse. So match both
+        ** values in these expressions.  */
+        if( sCtx.iCol<0 ){
+          sCtx.iAltCol = iCol;
+        }else if( !HasRowid(pTab) ){
+          Index *pPk = sqlite3PrimaryKeyIndex(pTab);
+          if( pPk && pPk->nKeyCol==1 && pPk->aiColumn[0]==iCol ){
+            sCtx.iAltCol = -1;
+          }
+        }
         for(pIdx=sParse.pNewTable->pIndex; pIdx; pIdx=pIdx->pNext){
           sqlite3WalkExprList(&sWalker, pIdx->aColExpr);
         }
         for(pIdx=sParse.pNewIndex; pIdx; pIdx=pIdx->pNext){
           sqlite3WalkExprList(&sWalker, pIdx->aColExpr);
         }
+        sCtx.iAltCol = sCtx.iCol;
 #ifndef SQLITE_OMIT_GENERATED_COLUMNS
         for(i=0; i<sParse.pNewTable->nCol; i++){
           Expr *pExpr = sqlite3ColumnExpr(sParse.pNewTable,
```

**Versions:** trunk d6f1df8c0bbc (2026-10-23) and every release back to 3.25.0, which added RENAME COLUMN.
