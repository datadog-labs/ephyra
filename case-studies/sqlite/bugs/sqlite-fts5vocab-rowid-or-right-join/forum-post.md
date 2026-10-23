**Title:** FTS5: fts5vocab rowids restart in every scan, so `term = ? OR term < ?` and RIGHT JOIN return wrong rows

```sql
CREATE VIRTUAL TABLE ft USING fts5(x);
INSERT INTO ft VALUES('apple banana');
CREATE VIRTUAL TABLE v USING fts5vocab(ft, 'row');
SELECT term FROM v WHERE term = 'banana' OR term < 'c';
SELECT t.a, v.term FROM (SELECT 'banana' AS a) t RIGHT JOIN v ON v.term = t.a;
```

Trunk d6f1df8c0bbc (2026-10-23) prints:

```
banana
banana
banana|banana
|banana
```

Expected (the first two rows in either order):

```
banana
apple
banana|banana
|apple
```

`fts5VocabRowidMethod()` returns a counter that xFilter resets to 0, so a scan that starts at a term (`term = ?`, `term >= ?`) numbers its rows from 1. The MULTI-INDEX OR and the RIGHT JOIN identify rows of v across scans by rowid, so apple (rowid 1 in the full scan) is taken for banana (rowid 1 in the scan for `term = 'banana'`).

The patch below makes every rowid equal to the row's position in a full scan: xFilter saves the start term, and the first xRowid call counts the rows before it. It changes a different part of `fts5_vocab.c` than the patch in our fts5vocab UTF-16 report (https://sqlite.org/forum/forumpost/b0c03988e8), and the two apply in either order. fts4aux has the same problem in `fts3auxRowidMethod()`, which the patch does not change.

```diff
--- a/ext/fts5/fts5_vocab.c
+++ b/ext/fts5/fts5_vocab.c
@@ -64,6 +64,9 @@
 
   int nLeTerm;                    /* Size of zLeTerm in bytes */
   char *zLeTerm;                  /* (term <= $zLeTerm) paramater, or NULL */
+  int nGeTerm;                    /* Size of zGeTerm in bytes */
+  char *zGeTerm;                  /* First term visited by xFilter, or NULL */
+  i64 iRowidOff;                  /* Add to rowid for xRowid, or -1 */
   int colUsed;                    /* Copy of sqlite3_index_info.colUsed */
 
   /* These are used by 'col' tables only */
@@ -418,6 +421,10 @@
   sqlite3_free(pCsr->zLeTerm);
   pCsr->nLeTerm = -1;
   pCsr->zLeTerm = 0;
+  sqlite3_free(pCsr->zGeTerm);
+  pCsr->nGeTerm = 0;
+  pCsr->zGeTerm = 0;
+  pCsr->iRowidOff = 0;
   pCsr->bEof = 0;
   pCsr->iCol = 0;
   pCsr->iInstPos = 0;
@@ -709,6 +716,20 @@
     }
   }
 
+  if( rc==SQLITE_OK && zTerm ){
+    /* The scan starts at zTerm instead of at the first term in the index.
+    ** Save a copy of it so that xRowid can find the number of rows that
+    ** precede it in a full scan.  */
+    pCsr->zGeTerm = sqlite3_malloc64((i64)nTerm+1);
+    if( pCsr->zGeTerm==0 ){
+      rc = SQLITE_NOMEM;
+    }else{
+      memcpy(pCsr->zGeTerm, zTerm, nTerm+1);
+      pCsr->nGeTerm = nTerm;
+      pCsr->iRowidOff = -1;
+    }
+  }
+
   if( rc==SQLITE_OK ){
     Fts5Index *pIndex = pCsr->pFts5->pIndex;
     rc = sqlite3Fts5IndexQuery(pIndex, zTerm, nTerm, f, 0, &pCsr->pIter);
@@ -805,18 +826,60 @@
   return SQLITE_OK;
 }
 
+/*
+** Cursor pCsr was filtered by a "term = ?" or "term >= ?" constraint, so
+** that its scan started at term pCsr->zGeTerm. Set pCsr->iRowidOff to the
+** number of rows that precede that term in a full scan of the table.
+*/
+static int fts5VocabFindRowidOff(Fts5VocabCursor *pCsr){
+  int nCol = pCsr->pFts5->pConfig->nCol;
+  i64 nByte = nCol * sizeof(i64)*2 + sizeof(Fts5VocabCursor);
+  Fts5VocabCursor *p;
+  int rc = SQLITE_OK;
+
+  p = (Fts5VocabCursor*)sqlite3Fts5MallocZero(&rc, nByte);
+  if( p ){
+    i64 iFirst;
+    p->base.pVtab = pCsr->base.pVtab;
+    p->pFts5 = pCsr->pFts5;
+    p->aCnt = (i64*)&p[1];
+    p->aDoc = &p->aCnt[nCol];
+    rc = fts5VocabFilterMethod((sqlite3_vtab_cursor*)p, 0, 0, 0, 0);
+    iFirst = p->rowid;
+    while( rc==SQLITE_OK && p->bEof==0 ){
+      int nCmp = MIN(p->term.n, pCsr->nGeTerm);
+      int bCmp = memcmp(p->term.p, pCsr->zGeTerm, nCmp);
+      if( bCmp>0 || (bCmp==0 && p->term.n>=pCsr->nGeTerm) ) break;
+      rc = fts5VocabNextMethod((sqlite3_vtab_cursor*)p);
+    }
+    if( rc==SQLITE_OK ) pCsr->iRowidOff = p->rowid - iFirst;
+    fts5VocabCloseMethod((sqlite3_vtab_cursor*)p);
+  }
+  return rc;
+}
+
 /* 
 ** This is the xRowid method. The SQLite core calls this routine to
 ** retrieve the rowid for the current row of the result set. The
 ** rowid should be written to *pRowid.
+**
+** The rowid of a row is its position in a full scan of the table. The
+** core relies on a row having the same rowid in every scan, for example
+** to remove duplicates from the scans of an OR constraint, so a scan
+** that starts at a term other than the first adds the number of rows
+** that precede that term.
 */
 static int fts5VocabRowidMethod(
   sqlite3_vtab_cursor *pCursor, 
   sqlite_int64 *pRowid
 ){
   Fts5VocabCursor *pCsr = (Fts5VocabCursor*)pCursor;
-  *pRowid = pCsr->rowid;
-  return SQLITE_OK;
+  int rc = SQLITE_OK;
+  if( pCsr->iRowidOff<0 ){
+    rc = fts5VocabFindRowidOff(pCsr);
+  }
+  *pRowid = pCsr->rowid + pCsr->iRowidOff;
+  return rc;
 }
 
 int sqlite3Fts5VocabInit(Fts5Global *pGlobal, sqlite3 *db){
```

**Versions:** wrong in 3.46.0, 3.51.0, 3.54.1 and trunk d6f1df8c0bbc (2026-10-23).
