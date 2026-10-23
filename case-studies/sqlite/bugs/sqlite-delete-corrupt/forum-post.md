**Title:** DELETE reports "database disk image is malformed" on a database that passes integrity_check (WITHOUT ROWID, tiny key)

The script below builds a 12-row database using only SQL, and `PRAGMA integrity_check` reports `ok`. After the first DELETE, `integrity_check` reports `Fragmentation of 1 bytes reported as 0 on page 2`. The third DELETE then fails with SQLITE_CORRUPT.

```sql
PRAGMA page_size=512;
CREATE TABLE t(c PRIMARY KEY) WITHOUT ROWID;
INSERT INTO t VALUES (hex(zeroblob(35)));
INSERT INTO t VALUES (hex(zeroblob(36)));
INSERT INTO t VALUES (hex(zeroblob(37)));
INSERT INTO t VALUES (hex(zeroblob(38)));
INSERT INTO t VALUES (hex(zeroblob(44)));
INSERT INTO t VALUES (hex(zeroblob(45)));
INSERT INTO t VALUES (hex(zeroblob(46)));
INSERT INTO t VALUES (hex(zeroblob(47)));
INSERT INTO t VALUES (x'');
INSERT INTO t VALUES (zeroblob(94));
INSERT INTO t VALUES (zeroblob(95));
INSERT INTO t VALUES (zeroblob(96));
PRAGMA integrity_check;
DELETE FROM t WHERE c = zeroblob(94);
DELETE FROM t WHERE c = hex(zeroblob(44));
DELETE FROM t WHERE c = x'';
```

Output from the 3.53.4 CLI, built from the release amalgamation with default options (`sqlite3 :memory: < repro.sql`):

```
ok
Error near line 18: database disk image is malformed
```

The same script fails with 3.46.0, 3.50.2 and 3.53.2 (default builds), and with the macOS system `sqlite3` (3.51.0, which reserves 12 bytes per page). It also fails on file databases. The failure depends on page layout, so other sizes or build options may need other key lengths.

**What happens.** The keys are texts of '0' characters (`hex(zeroblob(n))` is 2n characters long), the empty blob, and blobs of zero bytes. Texts sort before blobs, keys of one type sort by length, and `x''` sorts between the longest text and the shortest non-empty blob. Inserting them in order gives a two-level index b-tree: a root holding two dividers, the 88-character text (cell 0) and `zeroblob(94)` (cell 1), over three leaves. `x''` is the last entry of the middle leaf. I traced the three DELETEs with a modified 3.53.4. The offsets are on the root page of the default build.

The root page at byte level, from offset 408, where `x''` lands. `00 00 00 04` is the cell's left-child page number (page 4). `02` is the payload size, and `02 0c` is the record of `x''`. Byte 415 is the extra byte; it holds the leaf's padding (`00`).

```
1. DELETE … zeroblob(94): x'' moves up into the root

  408                 412  413  414  415  416
  ┌───────────────────┬────┬────┬────┬────┬─────────────────────┐
  │    00 00 00 04    │ 02 │ 02 │ 0c │ 00 │       text 88       │
  └───────────────────┴────┴────┴────┴────┴─────────────────────┘
  ╰─ x'' cell as parsed: 7 bytes ────╯
  ╰─ allocated by insertCell: 8 bytes ────╯

2. DELETE … hex(zeroblob(44)): a shorter cell replaces text 88

                                          416       428
  ┌───────────────────┬────┬────┬────┬────┬─────────┬───────────┐
  │    00 00 00 04    │ 02 │ 02 │ 0c │ 00 │  free   │  text 76  │
  └───────────────────┴────┴────┴────┴────┴─────────┴───────────┘

3. DELETE … x'': freeing the cell meets the extra byte

  ┌──────────────────────────────────┬────┬─────────┬───────────┐
  │       x'' (7 bytes) freed        │ 00 │  free   │  text 76  │
  └──────────────────────────────────┴────┴─────────┴───────────┘
                                       ↑ byte 415: counted as a
                                         fragment; header says 0
```

Step by step:

1. `DELETE … zeroblob(94)` removes root cell 1 (102 bytes at [314,416)). In `sqlite3BtreeDelete`, the largest entry of its left subtree, `x''`, moves up to replace it. On the leaf, that cell is 3 bytes: a payload size of 2, then the record `02 0c`. `cellSizePtrIdxLeaf` pads it to 4 (`if( nSize<4 ) nSize = 4;`). `insertCell(pPage, iCellIdx, pCell-4, nCell+4, pTmp, n)` therefore allocates 8 bytes, from the top of the unallocated area. The freed slot was at the start of the content area, so it rejoined that area, and the 8 bytes land at [408,416), right against cell 0. On the interior page the cell parses as 7 bytes (a 4-byte child pointer plus the 3 bytes). The byte at 415 belongs to no cell and is not counted at `hdr+7`.
2. `DELETE … hex(zeroblob(44))` removes root cell 0 (96 bytes at [416,512)). Its replacement, the 76-character text, needs only 84 bytes, carved from the end: [428,512). That leaves a free block at [416,428), starting right after the stray byte.
3. `DELETE … x''` frees root cell 1, the 7 bytes at [408,415). `freeSpace` merges them with the free block at 416, counts the 1-byte gap as a fragment, and finds `nFrag > data[hdr+7]`. It returns SQLITE_CORRUPT_PAGE (sqlite3.c line 75212 in the 3.53.4 amalgamation).

`balance_nonroot` already handles the opposite direction: it pads cells smaller than 4 bytes ("Do not allow any cells smaller than 4 bytes"). The interior-replacement path in `sqlite3BtreeDelete` does not undo the leaf padding. This is the byte `integrity_check` reports after step 1. Trunk `btree.c` has the same code.

In general, this needs an index b-tree whose leaf cells can be 3 bytes: a WITHOUT ROWID table whose primary key is one column, holding a key like `0`, `1`, `''` or `x''`. Entries in an ordinary index include the rowid, so they are at least 4 bytes; I could not trigger it that way. A DELETE must move that key up into an interior page. Free space must later start right after the stray byte, as when a neighbouring divider is replaced by a smaller one. Then that key is freed. A small page size makes the multi-level layout easy to reach with few rows.

We found this with a property-based test generator for SQLite database files and SQL statements. Its random failing case was reduced and then rebuilt as the script above.
