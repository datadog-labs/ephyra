**Title:** FTS5 detail=none/column: snippet() fails with "database disk image is malformed", highlight() and bm25() see matches in UNINDEXED columns

```sql
CREATE VIRTUAL TABLE t1 USING fts5(a, b UNINDEXED, detail=none);
INSERT INTO t1 VALUES ('one two zeta three four', 'x y zeta');

SELECT snippet(t1, -1, '[', ']', '..', 3) FROM t1('zeta');
SELECT highlight(t1, 1, '[', ']') FROM t1('zeta');
```

Output from trunk (check-in 185ffd27064b, 2026-10-20 13:10), built from the source tarball with --enable-fts5. 3.53.4 and 3.51.0 give the same:

```
Error near line 4: database disk image is malformed
x y [zeta]
```

Expected, and what the same table with detail=full prints:

```
one two [zeta]..
x y zeta
```

The database is fine (`INSERT INTO t1(t1) VALUES('integrity-check')` and PRAGMA integrity_check pass); the SQLITE_CORRUPT comes from snippet() itself. An UNINDEXED column is not in the full-text index, so a query cannot match it, and with detail=full the auxiliary functions see no phrase instances in it. With detail=none or detail=column they do: xInst() reports an instance of 'zeta' in the UNINDEXED column, so highlight() of that column marks it, bm25() scores it (with a third row ('five', 'six') in the tables, bm25() differs between detail=none and detail=full for both matching rows; with the patch below it is the same), and snippet() with column -1 fails as soon as the term is not the UNINDEXED value's first token (a value 'zeta y' works). It does not depend on the snippet length: 64 tokens fail the same way. snippet() of column 0 works. With ORDER BY rank, `t1('zeta NOT x')` returns the row with no instances at all (the NOT is re-evaluated on the rebuilt lists, which have `x` from b), and with `zeta OR x` the phrase `x`, which the index says is not in the row, gets an instance in b. The wrong instances come from the API, so custom auxiliary functions and matchinfo-style ones see them too: xInstCount/xInst, xPhraseFirst/Next, xInstToken, and xPhraseFirstColumn with detail=none (with detail=column it reads the index and is right, except with ORDER BY rank). External-content tables, columnsize=0, prefix queries and the trigram tokenizer show the same; contentless tables do not, as they report no instances with detail other than full.

**Cause, as far as we can tell.** With detail other than full, FTS5 builds the position lists for the auxiliary functions by tokenizing the row's text: `fts5CsrPoslist()` (fts5_main.c) runs `sqlite3Fts5ExprPopulatePoslists()` over every column, `pConfig->nCol` of them, including the UNINDEXED ones, so a phrase found in the UNINDEXED column gets an instance there. `fts5ApiColumnSize()` reports 0 tokens for an UNINDEXED column (it only counts the indexed ones). `fts5SnippetFunction()` (fts5_aux.c) checks each instance against the column size, `if( io>nDocsize ) rc = FTS5_CORRUPT;`, and 'zeta' at offset 2 of a column of size 0 trips it. With detail=full the position lists come from the index, which has nothing for UNINDEXED columns.

Skipping UNINDEXED columns there, as the code a few functions down that computes column sizes does, gives the expected output above, and the same bm25() values as with detail=full:

```diff
--- a/ext/fts5/fts5_main.c
+++ b/ext/fts5/fts5_main.c
@@ -2340,6 +2340,7 @@
       for(i=0; i<pConfig->nCol && rc==SQLITE_OK; i++){
         const char *z = 0;
         int n = 0; 
+        if( pConfig->abUnindexed[i] ) continue;
         rc = fts5TextFromStmt(pConfig, pCsr->pStmt, i, &z, &n);
         if( rc==SQLITE_OK ){
           rc = sqlite3Fts5ExprPopulatePoslists(
```

This is offered as a fix. The full patch, including regression tests in ext/fts5/test/fts5unindexed.test (a new section of 9 cases, run in each of the three detail modes), is in the attached sqlite-fts5-unindexed-detail-none-instances.patch.

Without the fix 16 of the 46 tests in fts5unindexed.test fail (detail=none and column); with it none do, all 149 files in ext/fts5/test (fault-injection tests included) give the same 5 pre-existing errors (fts5contentless3, fts5faultI) as without it, veryquick gives 29 errors in the same 7 files, and the tests run clean under ASan/UBSan. All four fixes from this batch were also applied together to trunk 185ffd27064b; every new test passes and veryquick shows no new failures.

**Versions** (FTS5 builds; our older release CLIs are built without FTS5): the same output in every one we tried, 3.46.0, 3.50.2 and 3.53.2 (amalgamations built with -DSQLITE_ENABLE_FTS5), 3.51.0 (the macOS /usr/bin/sqlite3, dated 2025-06-12), 3.53.4 (Homebrew) and trunk.

We found it with a property-based test generator for SQLite databases and queries: `snippet(vt0, -1, ...)` over a detail=none table with an UNINDEXED column failed with "database disk image is malformed".
