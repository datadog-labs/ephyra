**Title:** FTS5: phrases of a NEAR group that does not match a row report instances for that row, some left over from an earlier row

```sql
CREATE VIRTUAL TABLE t1 USING fts5(a, b);
INSERT INTO t1(rowid, a, b) VALUES (1, 'one', 'h e d c');
INSERT INTO t1(rowid, a, b) VALUES (2, 'r h c', 'p q s t u');
SELECT rowid, highlight(t1, 1, '[', ']') FROM t1('b : NEAR(h c, 2) OR r');
UPDATE t1 SET b = 'x' WHERE rowid = 2;
SELECT rowid, snippet(t1, 1, '[', ']', '..', 3) FROM t1('b : NEAR(h c, 2) OR r');
CREATE VIRTUAL TABLE t2 USING fts5(a);
INSERT INTO t2(a) VALUES ('r h d c');
SELECT highlight(t2, 0, '[', ']') FROM t2('NEAR(h "c d") OR r');
```

Output from trunk (check-in 3769f60be38b, 2026-10-21 16:05), built from the source tarball with --enable-fts5. 3.51.0 gives the same rows and the same error:

```
1|[h] e d [c]
2|p q s [t] u
Error near line 6: database disk image is malformed
1|[h] e d..
[r] [h] d c
```

Expected:

```
1|[h] e d [c]
2|p q s t u
1|[h] e d..
2|x
[r] h d c
```

Row 2 matches only through `r`, which is in column a. Column b of row 2 contains no query term, so nothing in it should be highlighted, and "t" is not a query term at all. The highlighted "t" is token 3 of column b, the position of "c" in row 1. In the second query, column b of row 2 has a single token, so `snippet()` sees an instance at token 3 of a 1-token column and fails with `SQLITE_CORRUPT` although the database is fine (the integrity-check passes). In t2, `NEAR(h "c d")` does not match ("c d" is not in the row), but "h" is still reported as an instance. Without the NEAR group, `t1('r')` and `t2('r')` give the expected rows.

Everything that reads instances is affected: `highlight()`, `snippet()`, `bm25()` and so `ORDER BY rank` (for t2, `bm25()` of the row is twice the value it has for the query `r`), and the `xInstCount()`, `xInst()`, `xPhraseFirst()` and `xPhraseFirstColumn()` APIs of custom auxiliary functions. The row has to be matched by another part of the query while a NEAR group with two or more phrases does not match it, as in `NEAR(...) OR x` or `x NOT NEAR(...)`, and the NEAR group has to have a column filter or contain a multi-token phrase. For other NEAR groups no phrase can fail on its own and the row is rejected by `fts5ExprNearIsMatch()`, which is fine. The same happens with locale=1 tables and other tokenizers; we found it on such a table.

The cause is in `fts5ExprNearTest()` in `ext/fts5/fts5_expr.c`. For detail=full, it walks the phrases of the NEAR group. If a phrase needs `fts5ExprPhraseIsMatch()` (a column filter, a multi-token phrase or colocated synonyms), that function rewrites the position list of the phrase for the current row, and the loop breaks as soon as one phrase does not match. The position lists of the phrases before it then hold their entries for the current row, and those of the phrases after it still hold the entries from the last row on which the loop reached them. The node is marked `bNomatch`, but `sqlite3Fts5ExprPoslist()` returns the position list of a phrase whenever its node points to the current rowid, without looking at `bNomatch`, so these entries reach `xInst()` and the sorter for `ORDER BY rank`. When `fts5ExprNearIsMatch()` rejects a row it leaves all position lists empty, so the break is the only path that leaves entries behind. In t1, phrase "h" of `b : NEAR(h c, 2)` has no entry in column b of row 2, so the loop stops there with "c" still holding its entry from row 1 (column 1, token 3).

The patch below empties the position lists of all phrases in the NEAR group when the loop breaks, which is what `fts5ExprNearIsMatch()` leaves when it rejects a row:

```diff
--- a/ext/fts5/fts5_expr.c
+++ b/ext/fts5/fts5_expr.c
@@ -929,7 +929,16 @@
       ){
         int bMatch = 0;
         rc = fts5ExprPhraseIsMatch(pNode, pPhrase, &bMatch);
-        if( bMatch==0 ) break;
+        if( bMatch==0 ){
+          /* The current row is not a match. Clear the position lists of
+          ** all phrases in the nearset, so that none of them report
+          ** entries for this row or retain entries from a previous row.  */
+          int j;
+          for(j=0; j<pNear->nPhrase; j++){
+            pNear->apPhrase[j]->poslist.n = 0;
+          }
+          break;
+        }
       }else{
         Fts5IndexIter *pIter = pPhrase->aTerm[0].pIter;
         fts5BufferSet(&rc, &pPhrase->poslist, pIter->nData, pIter->pData);
```

**Versions:** wrong in 3.46.0, 3.50.2, 3.51.0 (the macOS /usr/bin/sqlite3), 3.53.2 and trunk.

We found it with a property-based test generator for SQLite databases and queries: a query with `snippet()` over a locale=1 table failed with "database disk image is malformed" while the FTS5 integrity-check of the table passed.
