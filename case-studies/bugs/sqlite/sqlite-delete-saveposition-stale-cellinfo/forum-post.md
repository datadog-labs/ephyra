**Title:** "database disk image is malformed" from a write after a one-pass DELETE in the same statement (WITHOUT ROWID): the delete cursor keeps the deleted cell's CellInfo

```sql
PRAGMA page_size = 512;
CREATE TABLE t(a PRIMARY KEY, b, c) WITHOUT ROWID;
CREATE INDEX tc ON t(c);
INSERT INTO t VALUES (1, zeroblob(40), 1), (2, zeroblob(40), 2), (3, zeroblob(40), 3),
  (4, zeroblob(40), 4), (5, zeroblob(40), 5), (6, zeroblob(40), 6), (7, zeroblob(40), 7),
  (8, zeroblob(40), 8), (9, zeroblob(1300), 9);
CREATE TABLE u(x);
CREATE TRIGGER r AFTER INSERT ON u BEGIN
  DELETE FROM t WHERE c IS NULL OR a > 8;
  INSERT INTO t VALUES (0, 0, 0);
END;
PRAGMA integrity_check;
INSERT INTO u VALUES (1);
SELECT a, length(b) FROM t;
PRAGMA integrity_check;
```

Output from trunk (check-in 5bd093e261a7, 2026-10-16 18:11), built from the source tarball with default options (`sqlite3 :memory: < repro.sql`). 3.54.0 gives the same:

```
ok
Error near line 13: database disk image is malformed
1|40
2|40
3|40
4|40
5|40
6|40
7|40
8|40
9|1300
ok
```

Expected: no error, then `0|1`, `1|40` ... `8|40`, then `ok`. The database is not damaged; `integrity_check` is `ok` before and after (the failed statement is rolled back). `.log stderr` shows `database corruption at line 78085` of the amalgamation, the "Overflow chain ends prematurely" check of `accessPayload()`. The same happens with the default page size (for example rows of `zeroblob(400)` and one of `zeroblob(10000)`), on a file database, and with `UPDATE t SET b = 1 WHERE a = 1` in place of the INSERT.

**Cause, as far as we can tell.** The DELETE runs as a MULTI-INDEX OR, one pass: one cursor on t walks the rows, another one deletes them with `OP_Delete` and `OPFLAG_SAVEPOSITION` (delete.c, `eMode==ONEPASS_MULTI`). When the leaf does not need a rebalance after the delete, `sqlite3BtreeDelete()` takes the `bPreserve==2` path: it puts the cursor in `CURSOR_SKIPNEXT` on the neighbouring cell (`pCur->ix` unchanged, or `nCell-1` with `skipNext=-1` when the last cell went), but it does not clear `pCur->info` or `BTCF_ValidNKey`/`BTCF_ValidOvfl`, so the cursor still holds the CellInfo of the deleted cell. `sqlite3BtreeNext()` and `sqlite3BtreePrevious()` clear it before moving, but nothing else does. The INSERT then writes t through a third cursor, and `sqlite3BtreeInsert()` calls `saveAllCursors()`, which saves the SKIPNEXT cursor: `saveCursorPosition()` -> `saveCursorKey()` -> `sqlite3BtreePayloadSize()`/`sqlite3BtreePayload()`, and `getCellInfo()` does not parse the cell at `pCur->ix` because `info.nSize` is still set. So the "key" it saves is the deleted cell, read from page space `dropCell()` has freed and from an overflow chain `clearCell()` has put on the freelist. Here the first page of that chain became a freelist trunk, whose first four bytes (the next-trunk number) are 0, so the chain "ends" after one page: SQLITE_CORRUPT. When the freed pages are left as they were, the save succeeds and copies the deleted key instead; we did not find a wrong answer from that.

The save is meant to record the cell at `pCur->ix`: `btreeRestoreCursorPosition()` keeps `skipNext` when the seek finds that key exactly, which puts the cursor back in `CURSOR_SKIPNEXT` on the same cell. Discarding the cached CellInfo in the `bPreserve>1` branch does that, and fixes the script above:

```diff
--- a/src/btree.c
+++ b/src/btree.c
@@ -10063,6 +10063,13 @@
       }else{
         pCur->skipNext = 1;
       }
+      /* The cursor now points at a different cell.  Discard the cached
+      ** CellInfo and overflow page list of the deleted cell, so that a
+      ** later saveCursorPosition() saves the key of the cell at pCur->ix
+      ** rather than reading the freed space and overflow pages of the
+      ** deleted one. */
+      pCur->info.nSize = 0;
+      pCur->curFlags &= ~(BTCF_ValidNKey|BTCF_ValidOvfl);
     }else{
       rc = moveToRoot(pCur);
       if( bPreserve ){
```

With it, the without_rowid*, trigger*, view*, alter* and corrupt* tests and veryquick show no new failures. Take it as a pointer to the cause rather than a proposed fix. Rowid tables are not affected as far as we can see: `saveCursorKey()` saves only the rowid there, and the deleted rowid restores to the same place.

**Versions** (official builds, default options): 3.20.0, 3.25.0, 3.28.0, 3.30.0, 3.31.0, 3.33.0, 3.35.0, 3.37.0, 3.38.0, 3.39.0, 3.40.0, 3.50.4, 3.51.0 (macOS), 3.53.4, 3.54.0 and trunk all fail the same way (older ones word the error a little differently).

We found it with a property-based test generator for SQLite databases and queries: an `UPDATE` of a view whose INSTEAD OF trigger ran `DELETE FROM t2 WHERE (c3 BETWEEN NULL AND 'A' OR c0 > 1)` and then `UPDATE t2 SET c3 = <198-byte blob>` on a WITHOUT ROWID table, with 512-byte pages. It failed with "database disk image is malformed" on the first row the UPDATE wrote back.
