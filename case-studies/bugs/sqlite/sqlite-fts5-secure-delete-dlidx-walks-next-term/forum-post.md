**Title:** FTS5: after secure-delete removes the last rowids of a long doclist, a query with a rowid lower bound returns rows of the next term

```sql
CREATE VIRTUAL TABLE t USING fts5(x);
INSERT INTO t(t, rank) VALUES('pgsz', 32);
WITH s(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM s WHERE i<60)
  INSERT INTO t(rowid, x) SELECT i, iif(i<=50, 'a', 'b') FROM s;
INSERT INTO t(t, rank) VALUES('secure-delete', 1);
DELETE FROM t WHERE rowid = 50;
SELECT rowid FROM t('a') WHERE rowid >= 51;
SELECT count(*) FROM t('b NOT a');
INSERT INTO t(t) VALUES('integrity-check');
```

Output from trunk (check-in 185ffd27064b, 2026-10-20 13:10), built from the source tarball with --enable-fts5. 3.51.0 gives the same:

```
58
59
60
7
```

Expected: rows 51 to 60 contain only "b", so the first query returns no rows and "b NOT a" counts all ten of them:

```
10
```

That is what the same statements give without the secure-delete line, or with a `rebuild` after the DELETE. The integrity-check passes either way. The small pgsz only makes the table small: with the default page size, 8000 rows of "a" followed by 3000 rows of "b", after a secure delete of the last 2000 "a" rows, `t('a') WHERE rowid >= 8001` returns 2909 rows.

Affected are queries that advance a term's iterator to a given rowid: a term with `rowid >= N`, `rowid > N`, `rowid = N` or BETWEEN, a column filter such as `x:a` with a rowid bound, and AND, NOT and NEAR without any rowid constraint (in the example, `a AND b` and `NEAR(a b)` return 58, 59 and 60). Prefix queries are affected when a prefix index serves them. All detail modes, external content tables and tokendata=1 tables are affected. ORDER BY rowid DESC, plain scans of a term, and tables with `contentless_delete=1` are not. It needs secure-delete: without it, a DELETE leaves the leaves of the old segment as they are, and a merge writes a new doclist-index.

**Cause, as far as we can tell.** `fts5SegIterNextFrom()` in `ext/fts5/fts5_index.c` moves a single-term segment iterator forward to the first rowid at or after a target. It uses the doclist-index to find the last page whose first rowid is smaller than the target and passes that page to `fts5SegIterGotoPage()`. Secure-delete edits leaves in place and does not update the doclist-index, so the page may no longer hold a rowid of the doclist. `fts5SegIterGotoPage()` allows for that and moves on to the next page whose first-rowid field is set, but it does not look at whether a page has a term on it. A page with a term and no rowid before it is where the doclist ends. The pages after it belong to the next term, so the iterator continues in that term's doclist and reports its rowids for "a".

In the example, the doclist-index for "a" is `2(10) 3(20) 4(30) 5(40) 6(50)` (page, first rowid). Before the DELETE, leaf 6 is `0005001F 02 32 0202 023062 33 ...`: the first-rowid field is 5, where rowid 50 (0x32) is, followed by the term "b". After the DELETE it is `0000001C 02 023062 33 ...`, with no rowid before the term and the field set to 0. For `rowid >= 51` the doclist-index gives leaf 6. `fts5SegIterGotoPage()` skips it because the field is 0 and stops on leaf 7, `0005000E 02 3A ...`, whose first rowid, 58 (0x3A), belongs to "b". Rowids 59 and 60 follow on the same leaf.

The rest of the code expects such stale doclist-index entries in secure-delete tables: the integrity-check accepts an entry for a page without a rowid, or with a larger first rowid, when the table uses secure-delete, and `fts5SegIterReverse()` does not use the doclist-index for these tables at all. So the reader looks like the side to fix.

The patch below makes `fts5SegIterGotoPage()` stop at a page that has a term but no rowid before it, and leave the iterator at EOF, as `fts5SegIterNext()` does when it reaches such a page. It gives the expected output above, also for tables that are already in this state:

```diff
--- a/ext/fts5/fts5_index.c
+++ b/ext/fts5/fts5_index.c
@@ -3157,6 +3157,15 @@
         }
         break;
       }
+      if( !fts5LeafIsTermless(pIter->pLeaf) ){
+        /* The page has a term but no rowid before it, so the doclist ends
+        ** on this page and has no more rowids. This happens if the last
+        ** rowids of the doclist were removed by secure-delete, which does
+        ** not update the doclist-index. The iterator is now at EOF.  */
+        fts5DataRelease(pIter->pLeaf);
+        pIter->pLeaf = 0;
+        break;
+      }
     }
   }
 }
```

This is separate from our earlier secure-delete reports and from the tokendata=1 doclist-index report: it needs neither tokendata=1 nor a merge, it is in a different function, and none of those patches changes the output above. The patches apply together in either order.

**Versions:** wrong in 3.46.0, 3.50.2, 3.51.0 (the macOS /usr/bin/sqlite3, dated 2025-06-12), 3.53.2 and trunk.

We found it with a randomized differential test that compares FTS5 tables with an ordinary table, written while checking the patch for a related secure-delete report.
