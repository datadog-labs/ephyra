**Title:** FTS5: with detail=none, secure-delete of a rowid that was replaced while secure-delete was off corrupts the index

```sql
CREATE VIRTUAL TABLE t USING fts5(x, detail=none);
INSERT INTO t(rowid, x) VALUES (1, 'a');
INSERT OR REPLACE INTO t(rowid, x) VALUES (1, 'a');
INSERT INTO t(t, rank) VALUES ('secure-delete', 1);
DELETE FROM t WHERE rowid = 1;
SELECT count(*) FROM t;
SELECT rowid FROM t WHERE t MATCH 'a';
INSERT INTO t(t) VALUES ('integrity-check');
```

Output from trunk (check-in 3769f60be38b, 2026-10-21 16:05), built from the source tarball with --enable-fts5. 3.51.0 gives the same rows and fails the integrity-check with "database disk image is malformed":

```
0
1
5
6
Error near line 8: fts5: checksum mismatch for table "t"
```

Expected: the table is empty, so the MATCH returns no rows and the integrity-check passes:

```
0
```

That is what the same statements give with detail=full or detail=column, with secure-delete switched on before the first INSERT, or with `INSERT INTO t(t) VALUES ('rebuild')` run before secure-delete is switched on. Rowids 5 and 6 never existed.

Only detail=none tables are affected, including external content tables and tables with a prefix index (there the integrity-check reports "corruption found reading blob"). Tables with `contentless_delete=1` are not affected. The trigger is a doclist entry that was written to disk with the delete flag set while secure-delete was off: a rowid that was already in an earlier segment and was then deleted and inserted again in one transaction (INSERT OR REPLACE, UPDATE, or DELETE followed by INSERT). If secure-delete is later switched on and that rowid is deleted, by DELETE, UPDATE or INSERT OR REPLACE, the leaf is damaged. Depending on the position of the entry in the doclist, MATCH then returns rowids that never existed, still returns the deleted rowid, or both: with rows 1 to 5, replacing rowid 3 and then deleting it under secure-delete, `MATCH 'a'` returns 1, 2, 3, 4, 5, 7 and 8.

The cause is in `fts5DoSecureDelete()` in `ext/fts5/fts5_index.c`. In detail=none mode an entry is a rowid varint followed by no flag bytes (a plain entry), one 0x00 byte (delete marker) or two 0x00 bytes (delete, then insert). When the function is called, `pSeg->iLeafOffset` already points past the flag bytes of the entry to remove, because `fts5SegIterLoadNPos()` consumed them. The loop that looks for the start of the entry runs while `iSOP<pSeg->iLeafOffset` and assumes that only the rowid varint comes before `iLeafOffset`, so when the entry has flag bytes it skips them as if they belonged to an earlier entry and takes the bytes after them as the entry to remove. In the example the leaf of the second segment is `0000000A 023061 01 0000 04` (header, term "0a", rowid 1, flags 00 00, page footer 04). The loop steps over 00 00 and reads the footer byte 04 as a rowid delta. Because the entry has the delete flag, the function then writes that delta back followed by 0x01, which is the delete marker for detail=full and detail=column (position list size 0 with the delete bit). The leaf becomes `0000000C 023061 01 0000 04 01 04`: rowid 1 with its flags unchanged, then deltas 4 and 1, which are rowids 5 and 6. Even with the right start, 0x01 is wrong for detail=none, where it reads as the next rowid delta; there the delete marker is a single 0x00 byte.

The patch below stops the loop at the end of the rowid varint of the entry being removed and writes 0x00 instead of 0x01 as the delete marker in detail=none mode. With it, the leaf in the example becomes `00000009 023061 01 00 04`, rowid 1 as a delete marker that still hides the entry for rowid 1 in the first segment. detail=full and detail=column are not changed:

```diff
--- a/ext/fts5/fts5_index.c
+++ b/ext/fts5/fts5_index.c
@@ -5295,8 +5295,10 @@
   **     + the rowid or delta rowid value for the entry, and
   **     + the size of the position list in bytes.
   **
-  ** Or, in detail=none mode, there is a single varint prior to 
-  ** pSeg->iLeafOffset - the rowid or delta rowid value.
+  ** Or, in detail=none mode, the rowid or delta rowid value for the
+  ** entry is followed by zero, one or two 0x00 flag bytes, and
+  ** pSeg->iLeafOffset is the offset following the flag bytes. There is
+  ** one flag byte if pSeg->bDel is set, two if pSeg->nPos is also set.
   **
   ** This block sets the following variables:
   **
@@ -5331,7 +5333,9 @@
     assert_nc( iSOP<=pSeg->iLeafOffset );
 
     if( bDetailNone ){
-      while( iSOP<pSeg->iLeafOffset ){
+      /* Offset of the first byte following the rowid varint */
+      const int iEnd = pSeg->iLeafOffset - (pSeg->bDel ? 1+pSeg->nPos : 0);
+      while( iSOP<iEnd ){
         if( aPg[iSOP]==0x00 ) iSOP++;
         if( aPg[iSOP]==0x00 ) iSOP++;
         iStart = iSOP;
@@ -5396,7 +5400,7 @@
 
   if( pSeg->bDel ){
     iOff += sqlite3Fts5PutVarint(&aPg[iOff], iDelta);
-    aPg[iOff++] = 0x01;
+    aPg[iOff++] = (bDetailNone ? 0x00 : 0x01);
   }else if( bLastInDoclist==0 ){
     if( iNextOff!=iPgIdx ){
       u64 iNextDelta = 0;
```

**Versions:** wrong in 3.46.0, 3.50.2, 3.51.0 (the macOS /usr/bin/sqlite3, dated 2025-06-12), 3.53.2 and trunk.

We found it with a property-based test generator for SQLite databases and queries: after an UPDATE of a replaced rowid in a detail=none table with secure-delete switched on, the FTS5 integrity-check reported corruption.
