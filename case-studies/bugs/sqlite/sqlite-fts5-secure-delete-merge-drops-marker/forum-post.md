**Title:** FTS5: with secure-delete on, a merge can drop a delete marker, and a later secure delete makes an old entry for the rowid match again

```sql
CREATE VIRTUAL TABLE t USING fts5(x);
INSERT INTO t(t, rank) VALUES ('pgsz', 32);
INSERT INTO t(rowid, x) VALUES (1, 'c');
INSERT INTO t(rowid, x) VALUES (2, 'a b d e f g h');
INSERT INTO t(t) VALUES ('optimize');
DELETE FROM t WHERE rowid = 1;
INSERT INTO t(t, rank) VALUES ('secure-delete', 1);
INSERT INTO t(rowid, x) VALUES (1, 'c');
INSERT INTO t(t, rank) VALUES ('usermerge', 2);
INSERT INTO t(t, rank) VALUES ('merge', 1);
DELETE FROM t WHERE rowid = 1;
SELECT count(*) FROM t;
SELECT rowid FROM t WHERE t MATCH 'c';
INSERT INTO t(t) VALUES ('integrity-check');
```

Output from trunk (check-in 3769f60be38b, 2026-10-21 16:05), built from the source tarball with --enable-fts5. 3.51.0 gives the same rows and fails the integrity-check with "database disk image is malformed":

```
1
1
Error near line 14: fts5: checksum mismatch for table "t"
```

Expected: only row 2 is left and it does not contain "c", so the MATCH returns no rows and the integrity-check passes:

```
1
```

That is what the same statements give without the `merge` line, or with secure-delete switched on before the first INSERT.

All detail modes are affected (detail=full, detail=column and detail=none give the same output), as are tables with a prefix index and external content tables. Tables with `contentless_delete=1` are not affected. The layout that triggers it:

1. An older segment, on a higher-numbered level than the one being merged, holds an entry for term "c" and rowid 1.
2. With secure-delete off, rowid 1 is deleted. A newer segment gets a delete marker for "c", rowid 1.
3. With secure-delete on, rowid 1 gets "c" again. A newer segment gets a plain entry for "c", rowid 1, with no delete flag.
4. A merge (automerge, crisismerge or the `merge` command, complete or incremental) combines the segments from steps 2 and 3 into a segment that is not the oldest one in the index.
5. With secure-delete on, rowid 1 is deleted again, by DELETE, UPDATE or INSERT OR REPLACE.

After step 5, the entry from step 1 is visible again: MATCH returns the rowid, and if the rowid now holds other text, it matches terms it no longer contains. The random case that found it hit step 4 through a crisis merge of 16 level-0 segments in the same statement as step 3.

The cause is the merge in `fts5IndexMergeLevel()` in `ext/fts5/fts5_index.c`. When several input segments hold an entry for the same term and rowid, the multi-iterator skips all but the newest (`fts5MultiIterDoCompare()`), and only the newest is written, with its own delete flag. Delete markers are kept when the output segment is not the oldest, because older segments may still hold entries for the rowid. But if the newest entry is a plain entry and an older input entry is the delete marker, the marker is dropped and the output holds a plain entry. For queries that is fine: the plain entry hides the older entry. For secure-delete it is not: `fts5DoSecureDelete()` replaces an entry with a delete marker only if the entry has the delete flag, and removes an entry without it completely. After the merge, nothing records that an older segment holds an entry for the rowid. The same happens when an incremental merge (automerge, or `merge` with a small page count) stops at a term boundary: the multi-iterator has already skipped the older entries for the next term that duplicate a newer one, and `fts5TrimSegments()` drops them from their input segments, so a delete marker skipped this way is gone when the merge resumes. This can happen even if the output segment is the oldest one, because the entry that the marker hides can be in another input segment that the merge has not reached yet.

In the example, the old segment (level 1) holds rowid 1 for term "c". Before the merge, level 0 holds a segment with `01 01` (rowid 1, size 0 with the delete bit) and a newer one with `01 02 02` (rowid 1, size 1, no delete bit). The merge writes `0000000A 023063 01 02 02 04` to a second level-1 segment, a plain entry. The final DELETE removes it completely, and the leaf becomes `00000004`, an empty page, so the entry in the old segment is visible again.

The patch below records, when the multi-iterator skips an older entry for the same term and rowid, whether that entry, or an entry it superseded, has the delete flag set. `fts5IndexMergeLevel()` then sets the delete flag on the output entry if the output segment is not the oldest, and does not stop an incremental merge while such an entry has not been written yet. With it, the merge in the example writes `01 03 02` (size 1 with the delete bit), and the DELETE leaves `00000009 023063 01 01 04`, a delete marker that still hides the entry in the old segment:

```diff
--- a/ext/fts5/fts5_index.c
+++ b/ext/fts5/fts5_index.c
@@ -557,6 +557,7 @@
   i64 iRowid;                     /* Current rowid */
   int nPos;                       /* Number of bytes in current position list */
   u8 bDel;                        /* True if the delete flag is set */
+  u8 bDupDel;                     /* True if a skipped duplicate had bDel */
 };
 
 static int fts5IndexCorruptRowid(Fts5Index *pIdx, i64 iRowid){
@@ -3105,6 +3106,11 @@
       assert_nc( i2!=0 );
       pRes->bTermEq = 1;
       if( p1->iRowid==p2->iRowid ){
+        /* p2 is an older entry for the same term and rowid. It is about
+        ** to be skipped. If it, or an entry it superseded, has the delete
+        ** flag set, record this on p1. See fts5IndexMergeLevel(). */
+        p1->bDupDel |= (p2->bDel | p2->bDupDel);
+        p2->bDupDel = 0;
         return i2;
       }
       res = ((p1->iRowid > p2->iRowid)==pIter->bRev) ? -1 : +1;
@@ -3406,6 +3412,7 @@
     int bNewTerm = 0;
     Fts5SegIter *pSeg = &pIter->aSeg[iFirst];
     assert( p->rc==SQLITE_OK );
+    pSeg->bDupDel = 0;
     if( bUseFrom && pSeg->pDlidx ){
       fts5SegIterNextFrom(p, pSeg, iFrom);
     }else{
@@ -4860,18 +4867,35 @@
     int nPos;                     /* position-list size field value */
     int nTerm;
     const u8 *pTerm;
+    int bDel;                     /* Delete flag for output entry */
 
     pTerm = fts5MultiIterTerm(pIter, &nTerm);
     if( nTerm!=term.n || fts5Memcmp(pTerm, term.p, nTerm) ){
       if( pnRem && writer.nLeafWritten>nRem ){
-        break;
+        /* Do not stop here if an entry not yet written superseded an
+        ** entry with the delete flag set. fts5TrimSegments() may remove
+        ** the superseded entry from its input segment, while an entry
+        ** it hides remains in an older input segment or outside of the
+        ** merge.  */
+        int ii;
+        for(ii=0; ii<pIter->nSeg; ii++){
+          if( pIter->aSeg[ii].bDupDel ) break;
+        }
+        if( ii==pIter->nSeg ) break;
       }
       fts5BufferSet(&p->rc, &term, nTerm, pTerm);
       bTermWritten =0;
     }
 
+    /* If the output is not the oldest segment and this entry supersedes
+    ** an entry with the delete flag set, set the delete flag on the
+    ** output entry as well. Otherwise, the delete marker is lost, and
+    ** a secure-delete may later remove the entry outright, exposing an
+    ** entry for the same rowid in an older segment.  */
+    bDel = pSegIter->bDel || (bOldest==0 && pSegIter->bDupDel);
+
     /* Check for key annihilation. */
-    if( pSegIter->nPos==0 && (bOldest || pSegIter->bDel==0) ) continue;
+    if( pSegIter->nPos==0 && (bOldest || bDel==0) ) continue;
 
     if( p->rc==SQLITE_OK && bTermWritten==0 ){
       /* This is a new term. Append a term to the output segment. */
@@ -4884,7 +4908,7 @@
     fts5WriteAppendRowid(p, &writer, fts5MultiIterRowid(pIter));
 
     if( eDetail==FTS5_DETAIL_NONE ){
-      if( pSegIter->bDel ){
+      if( bDel ){
         fts5BufferAppendVarint(&p->rc, &writer.writer.buf, 0);
         if( pSegIter->nPos>0 ){
           fts5BufferAppendVarint(&p->rc, &writer.writer.buf, 0);
@@ -4892,7 +4916,7 @@
       }
     }else{
       /* Append the position-list data to the output */
-      nPos = pSegIter->nPos*2 + pSegIter->bDel;
+      nPos = pSegIter->nPos*2 + bDel;
       fts5BufferAppendVarint(&p->rc, &writer.writer.buf, nPos);
       fts5ChunkIterate(p, pSegIter, (void*)&writer, fts5MergeChunkCallback);
     }
```

In detail=none tables this patch makes merges write entries with the delete flag in more cases, and secure-delete of such an entry is the case of the earlier report "FTS5: with detail=none, secure-delete of a rowid that was replaced while secure-delete was off corrupts the index". With this patch alone, the example with detail=none returns rowids 1, 5 and 6; with the patches for both reports it gives the expected output. The two patches touch different functions and apply in either order.

**Versions:** wrong in 3.46.0, 3.50.2, 3.51.0 (the macOS /usr/bin/sqlite3, dated 2025-06-12), 3.53.2 and trunk.

We found it with a property-based test generator for SQLite databases and queries: a randomized differential test that compares FTS5 tables in each detail mode with an ordinary table, written while checking the patch for the detail=none report, found a MATCH that returned a deleted row.
