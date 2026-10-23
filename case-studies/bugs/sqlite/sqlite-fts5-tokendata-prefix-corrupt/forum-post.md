**Title:** FTS5 tokendata=1: a prefix query followed by a term query in an open transaction reads freed memory

```sql
CREATE VIRTUAL TABLE t1 USING fts5(x, tokendata=1, prefix=1);
INSERT INTO t1 VALUES('vv aa');
BEGIN;
INSERT INTO t1 VALUES('vv aa');
SELECT rowid FROM t1 WHERE t1 MATCH 'v* AND aa';
```

Output from trunk d6f1df8c0bbc (2026-10-23), built with `--enable-fts5`:

```
1
Error near line 5: database disk image is malformed
```

Expected:

```
1
2
```

In `sqlite3Fts5IndexQuery()`, the iterator for `v*` (prefix index) points into the segment arrays of the cached `Fts5Structure`. The term query `aa` then goes to `fts5SetupTokendataIter()`, whose `fts5IndexFlush()` reallocates and frees that structure while the first iterator is still in use (AddressSanitizer: heap-use-after-free in `fts5SegIterNextPage()`). The error text depends on what is in the freed memory.

The patch below flushes pending terms at the start of every query on a tokendata=1 table, before any iterator is opened. With it, the script prints 1 and 2.

```diff
--- a/ext/fts5/fts5_index.c
+++ b/ext/fts5/fts5_index.c
@@ -7454,6 +7454,15 @@
       bTokendata = 0;
     }
 
+    /* fts5SetupTokendataIter() flushes the in-memory hash table to disk.
+    ** The flush frees the cached Fts5Structure object, which iterators
+    ** opened earlier by this function point into without holding a
+    ** reference (for example one opened for a prefix index query in the
+    ** same MATCH expression). So, on a tokendata=1 table, flush first.  */
+    if( pConfig->bTokendata ){
+      fts5IndexFlush(p);
+    }
+
     /* Figure out which index to search and set iIdx accordingly. If this
     ** is a prefix query for which there is no prefix index, set iIdx to
     ** greater than pConfig->nPrefix to indicate that the query will be
```

Also fails on 3.46.0, 3.50.2, 3.51.0 and 3.53.2.
