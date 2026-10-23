**Title:** FTS5: in a UTF-16 database, fts5vocab applies `term <` / `>` and ORDER BY term in UTF-8 order

```sql
PRAGMA encoding = 'UTF-16le';
CREATE VIRTUAL TABLE ft USING fts5(a);
INSERT INTO ft VALUES ('apple ĳs b');   -- ĳ is U+0133
CREATE VIRTUAL TABLE v USING fts5vocab(ft, 'row');
CREATE TABLE t(term TEXT);
INSERT INTO t SELECT term FROM v;

SELECT term FROM v WHERE term < 'b';
SELECT term FROM v WHERE +term < 'b';
SELECT term FROM v ORDER BY term;
SELECT term FROM t ORDER BY term;
```

Output from trunk (check-in 3769f60be38b, 2026-10-21 16:05), built from the source tarball with --enable-fts5. 3.51.0 and 3.54.1 give the same:

```
apple
apple
ĳs
apple
b
ĳs
ĳs
apple
b
```

Expected: `term < 'b'` returns the same two rows as `+term < 'b'`, and `ORDER BY term` gives the order of the ordinary table t:

```
apple
ĳs
apple
ĳs
ĳs
apple
b
ĳs
apple
b
```

Under the BINARY collation SQLite compares text with memcmp() in the database encoding. In UTF-16le, 'ĳs' (bytes 33 01 ...) sorts before 'apple' (61 00 ...), and `SELECT 'ĳs' < 'b'` returns 1. The fts5vocab table instead drops 'ĳs' from `term < 'b'` and returns `ORDER BY term` in UTF-8 order, while the same query with `+term`, or over the ordinary table, is right.

All three fts5vocab types (row, col and instance) are affected, for each of `<`, `<=`, `>` and `>=` on term, including a range that the optimizer pushes down from an outer query into a subquery or view over the vocabulary table, and for `ORDER BY term`. Since SQLite re-checks the constraints, the range forms return too few rows, never extra ones. In a UTF-16le database any term with a character at or above U+0100 can be affected (a bound with such a character can also drop ASCII terms: with the terms above, `term >= 'aĀ'` leaves out 'apple', which sorts after 'aĀ' in UTF-16le). In a UTF-16be database the order differs only between characters above U+FFFF and those in U+E000 to U+FFFF: with `tokenize=ascii` and the terms 'b', 'ﬀ' (U+FB00) and '😀' (U+1F600), `term < 'ﬀ'` leaves out '😀' and `ORDER BY term` gives 'b ﬀ 😀' instead of 'b 😀 ﬀ'. `term = ...` is right in every encoding for well-formed text, a comparison with another collation (for example `term < 'b' COLLATE nocase`) is not passed to the virtual table and is right, and UTF-8 databases are not affected. fts4aux has the same xBestIndex logic (`fts3auxBestIndexMethod()` in `ext/fts3/fts3_aux.c`) and gives the same wrong results for `term < 'b'` and `ORDER BY term` in a UTF-16le database; the patch below does not change it.

The cause is that the FTS5 index holds terms as UTF-8 and visits them in memcmp() order of those bytes. `fts5VocabBestIndexMethod()` takes `term` constraints `=`, `<`, `<=`, `>` and `>=` whenever their collation is BINARY, and sets orderByConsumed for `ORDER BY term`, saying that the rows come in term order. `fts5VocabFilterMethod()` reads the bounds with `sqlite3_value_text()`, which converts them to UTF-8, seeks to the lower bound with `sqlite3Fts5IndexQuery()`, and `fts5VocabNextMethod()` stops at the first term whose UTF-8 bytes compare greater than the upper bound. That is the BINARY order only when the database encoding is UTF-8. In a UTF-16 database the terms that satisfy a range are not a contiguous run in UTF-8 order, so converting the bounds to UTF-16 would not help; the scan has to cover all terms. Equality does not depend on the order, so it is fine.

The patch below reads the database text encoding with `PRAGMA encoding` when the fts5vocab table is connected (as FTS3 reads `PRAGMA page_size` in xConnect, and FTS5 already runs `PRAGMA data_version`), and when it is not UTF-8 makes xBestIndex leave range constraints on term and ORDER BY term to SQLite. Equality constraints are still used. Plans for UTF-8 databases do not change:

```diff
--- a/ext/fts5/fts5_vocab.c
+++ b/ext/fts5/fts5_vocab.c
@@ -51,6 +51,7 @@
   Fts5Global *pGlobal;            /* FTS5 global object for this database */
   int eType;                      /* FTS5_VOCAB_COL, ROW or INSTANCE */
   unsigned bBusy;                 /* True if busy */
+  int bUtf8;                      /* True if db text encoding is UTF-8 */
 };
 
 struct Fts5VocabCursor {
@@ -149,6 +150,28 @@
 }
 
 /*
+** Return true if the text encoding of database connection db is UTF-8.
+** Terms are stored in the FTS5 index in UTF-8 byte order. This matches
+** the order of the BINARY collation only if the database text encoding
+** is also UTF-8.
+*/
+static int fts5VocabIsUtf8(int *pRc, sqlite3 *db){
+  int bRet = 0;
+  if( *pRc==SQLITE_OK ){
+    sqlite3_stmt *pStmt = 0;
+    *pRc = sqlite3_prepare_v2(db, "PRAGMA encoding", -1, &pStmt, 0);
+    if( *pRc==SQLITE_OK ){
+      if( SQLITE_ROW==sqlite3_step(pStmt) ){
+        const char *zEnc = (const char*)sqlite3_column_text(pStmt, 0);
+        bRet = (zEnc && sqlite3_stricmp(zEnc, "UTF-8")==0);
+      }
+      *pRc = sqlite3_finalize(pStmt);
+    }
+  }
+  return bRet;
+}
+
+/*
 ** This function is the implementation of both the xConnect and xCreate
 ** methods of the FTS3 virtual table.
 **
@@ -200,18 +223,21 @@
     i64 nDb = strlen(zDb)+1; 
     i64 nTab = strlen(zTab)+1;
     int eType = 0;
+    int bUtf8 = 0;
     
     rc = fts5VocabTableType(zType, pzErr, &eType);
     if( rc==SQLITE_OK ){
       assert( eType>=0 && eType<ArraySize(azSchema) );
       rc = sqlite3_declare_vtab(db, azSchema[eType]);
     }
+    bUtf8 = fts5VocabIsUtf8(&rc, db);
 
     nByte = sizeof(Fts5VocabTable) + nDb + nTab;
     pRet = sqlite3Fts5MallocZero(&rc, nByte);
     if( pRet ){
       pRet->pGlobal = (Fts5Global*)pAux;
       pRet->eType = eType;
+      pRet->bUtf8 = bUtf8;
       pRet->db = db;
       pRet->zFts5Tbl = (char*)&pRet[1];
       pRet->zFts5Db = &pRet->zFts5Tbl[nTab];
@@ -271,11 +297,16 @@
 **
 ** are interpreted. Less-than and less-than-or-equal are treated 
 ** identically, as are greater-than and greater-than-or-equal.
+**
+** Terms are visited in UTF-8 byte order. If the database text encoding
+** is UTF-16, the BINARY collation orders text differently, so range
+** constraints and ORDER BY are left to the core in that case.
 */
 static int fts5VocabBestIndexMethod(
-  sqlite3_vtab *pUnused,
+  sqlite3_vtab *pVTab,
   sqlite3_index_info *pInfo
 ){
+  Fts5VocabTable *pTab = (Fts5VocabTable*)pVTab;
   int i;
   int iTermEq = -1;
   int iTermGe = -1;
@@ -283,8 +314,6 @@
   int idxNum = (int)pInfo->colUsed;
   int nArg = 0;
 
-  UNUSED_PARAM(pUnused);
-
   assert( (pInfo->colUsed & FTS5_VOCAB_COLUSED_MASK)==pInfo->colUsed );
 
   for(i=0; i<pInfo->nConstraint; i++){
@@ -292,6 +321,7 @@
     if( p->usable==0 ) continue;
     if( p->iColumn==0 && fts5VocabIsBinary(pInfo, i) ){    /* term column */
       if( p->op==SQLITE_INDEX_CONSTRAINT_EQ ) iTermEq = i;
+      if( pTab->bUtf8==0 ) continue;
       if( p->op==SQLITE_INDEX_CONSTRAINT_LE ) iTermLe = i;
       if( p->op==SQLITE_INDEX_CONSTRAINT_LT ) iTermLe = i;
       if( p->op==SQLITE_INDEX_CONSTRAINT_GE ) iTermGe = i;
@@ -322,7 +352,8 @@
   ** specifically - "ORDER BY term" or "ORDER BY term ASC" - set the
   ** sqlite3_index_info.orderByConsumed flag to tell the core the results
   ** are already in sorted order.  */
-  if( pInfo->nOrderBy==1 
+  if( pTab->bUtf8
+   && pInfo->nOrderBy==1 
    && pInfo->aOrderBy[0].iColumn==0 
    && pInfo->aOrderBy[0].desc==0
   ){
```

The encoding is read before the table object is allocated, so an error from the PRAGMA (for example `SQLITE_NOMEM`) is returned from xConnect without leaking it. The encoding of a database cannot change once it has a schema, and all attached databases share it, so reading it once per connection of the virtual table is enough.

**Versions**: wrong in 3.51.0 (the macOS /usr/bin/sqlite3, dated 2025-06-12), 3.54.1 (Homebrew) and trunk.

We found it with a property-based test generator for SQLite databases and queries: in a UTF-16 database, `term < 'replacement'` on an fts5vocab table and the same query with `+term` returned different rows.
