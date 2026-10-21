**Title:** FTS5 tokendata=1: a query for an absent term fails with "fts5: corruption found reading blob" after a long doclist

```sql
CREATE VIRTUAL TABLE f USING fts5(a, tokendata=1);
INSERT INTO f(f, rank) VALUES('pgsz', 64);
WITH s(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM s WHERE i<100)
INSERT INTO f(rowid, a) SELECT i, iif(i<100, 'u0', 'v') FROM s;
SELECT count(*) FROM f('u1');
```

Output from trunk (check-in 185ffd27064b, 2026-10-20 13:10), built from the source tarball with --enable-fts5. 3.54.1 gives the same; 3.51.0 says "database disk image is malformed":

```
Error near line 5: fts5: corruption found reading blob 206158430214 from table "f"
```

Expected, and what the same table without tokendata=1 prints:

```
0
```

The database is fine: `INSERT INTO f(f) VALUES('integrity-check')` and PRAGMA integrity_check pass, and `f('u0')`, `f('v')` and `f('u')` give 99, 1 and 0. The query fails when a segment does not contain the term, the term sorts after a term whose doclist is long enough to have a doclist-index (here 'u0', 99 rows), and the next larger term ('v') is the only term on a later leaf page. The small pgsz only makes the table small: with the default page size, 9999 rows of 'u0' and one 'v' fail the same way. It is not only about absent terms: after `INSERT INTO f(rowid, a) VALUES(200, 'u1')`, which goes into a second segment, `SELECT rowid FROM f('u1')` fails with the same error instead of returning 200. After `'merge'` steps, a term present in the only segment fails the same way when it has no doclist-index of its own (test 3.1.4 in the patch, which uses the origintext tokenizer).

**Cause, as far as we can tell.** A tokendata=1 query for a term opens each segment with `fts5SegIterSeekInit()` (fts5_index.c), called from `fts5SetupTokendataIter()` with FTS5INDEX_QUERY_SCAN and FTS5INDEX_QUERY_SCANONETERM: a >= search that is treated as a single-term iterator. It is the only caller that combines the two. `fts5SegIterSeekInit()` looks up the leaf page in %_idx and takes `bDlidx` from the same row. That flag says whether the last term on that page (iPg) has a doclist-index. When the term is absent and nothing on page iPg is larger, `fts5LeafSeek()` moves on to the first term on a later page. `fts5SegIterSeekInit()` still uses the flag of page iPg, and `fts5SegIterLoadDlidx()` then reads the doclist-index of the new page, which does not exist. Blob 206158430214 is the doclist-index rowid for segment 1, page 6; the doclist-index that exists is for page 1, where 'u0' is. Without tokendata=1 the search for a term is not a >= search and stops at EOF, and prefix queries do not use doclist-indexes.

Looking up the flag for the page the iterator ends up on gives the expected output above:

```diff
--- a/ext/fts5/fts5_index.c
+++ b/ext/fts5/fts5_index.c
@@ -2756,7 +2756,23 @@
       if( flags & FTS5INDEX_QUERY_DESC ){
         pIter->flags |= FTS5_SEGITER_REVERSE;
       }
-      if( bDlidx ){
+      if( pIter->iTermLeafPgno!=iPg ){
+        /* The bDlidx flag read from %_idx above applies to the last term
+        ** on page iPg. This is a >= search and fts5LeafSeek() has moved
+        ** the iterator to the first term on a later page, so look up the
+        ** flag for that page instead.  */
+        bDlidx = 0;
+        sqlite3_bind_int(pIdxSelect, 1, pSeg->iSegid);
+        sqlite3_bind_blob(pIdxSelect, 2, pIter->term.p, pIter->term.n,
+            SQLITE_STATIC
+        );
+        if( SQLITE_ROW==sqlite3_step(pIdxSelect) ){
+          bDlidx = (sqlite3_column_int(pIdxSelect, 0) & 0x0001);
+        }
+        p->rc = sqlite3_reset(pIdxSelect);
+        sqlite3_bind_null(pIdxSelect, 2);
+      }
+      if( bDlidx && p->rc==SQLITE_OK ){
         fts5SegIterLoadDlidx(p, pIter);
       }
       if( flags & FTS5INDEX_QUERY_DESC ){
```

When the term found is on a later page, the patch looks up that page's %_idx row with the same statement (the existing `fts5IdxSelectStmt()` pattern in the function) and takes the doclist-index flag from it, so no doclist-index is skipped. The extra lookup happens only after `fts5LeafSeek()` has already read a further leaf, and we could not measure its cost. Usually the term found does not match the query, as a matching term would have been found through %_idx. The only way a seek lands on a matching term on a later page is an incremental merge, which writes the first term of each resumed step to %_idx whole. The simpler fix, skipping the doclist-index when the page differs (`bDlidx && pIter->iTermLeafPgno==iPg`), is also correct, but makes a query that already works, `rowid>=` on such a term after incremental merge steps, about 7x slower.

This is offered as a fix. The full patch, including regression tests in ext/fts5/test/fts5tokendata.test (15 new cases in two sections), is in the attached sqlite-fts5-tokendata-missing-term-dlidx-corrupt.patch.

Without the fix 5 of the 49 tests in fts5tokendata.test fail; with it none do. With this patch and the one for the locale=1/contentless_unindexed=1 report we are posting at the same time applied together to trunk 185ffd27064b, all 149 files in ext/fts5/test (fault-injection tests included) give the same 5 pre-existing errors (fts5contentless3, fts5faultI) as without them, the fts5fault* results are unchanged, veryquick gives 29 errors in the same 7 files, and the new tests and the example run clean under ASan/UBSan. Each patch alone fixes only its own tests. In a differential fuzz of tokendata=1 tables with small pages and merges, trunk fails with the corruption error in about a third of the iterations, and the fix gives the same output as the simpler variant.

**Versions** (FTS5 builds): the same error in every one we tried, 3.46.0, 3.50.2 and 3.53.2 (amalgamations built with -DSQLITE_ENABLE_FTS5), 3.51.0 (the macOS /usr/bin/sqlite3), 3.54.1 (Homebrew) and trunk.

We found it with a property-based test generator for SQLite databases and queries: a MATCH on a tokendata=1 table with a small pgsz failed with "fts5: corruption found reading blob".
