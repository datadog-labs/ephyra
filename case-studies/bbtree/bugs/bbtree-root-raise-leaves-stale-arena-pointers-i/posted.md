# BBTreeMapFile::create writes the writer's heap addresses into the file's root page, so the same entries give different files

Posted as https://github.com/XiangpengHao/bbtree/issues/31 on 2026-10-28. The text below is the issue as posted.

---

This one seems like a bit of a strech at this point, so I'll stop the run. The tests are at https://github.com/datadog-labs/ephyra/tree/main/case-studies/bbtree in case you would be interested in using them in some capacity in the future.

## Summary

When a map with `String` (or `Vec<u8>`) values has enough entries that the writer's tree grows past one leaf, `BBTreeMapFile::create` writes heap addresses of the writing process into the file's root page. The file then depends on where the writer's allocations happened to lie, not only on the entries, and it carries the process's addresses to anyone who reads it.

`design/15-index-file.md` says a file map depends on its entries alone:

> A map of random inserts, two-thirds full, compacts the same as one built in order: a file map depends on the entries alone (tested). (line 59)

and that the writer's `ArenaBytes` pointers do not reach the file:

> `HeapPages` rewrites each key and value slot, the head's included, into a span, as it lays the bytes out. Slots past a leaf's entries become zeros, not stale pointers. (lines 171-174)

and `bbtree/src/file/tests.rs:253`: "the file map is compact, and holds no trace of the map's addresses."

## Reproducer

```rust
use std::collections::BTreeMap;
use bbtree_mmap::BBTreeMapFile;

fn main() {
    let out = std::env::args().nth(1).expect("output path");
    // 200 entries: enough that the writer's tree grows past one leaf.
    let map: BTreeMap<u64, String> = (0..200u64).map(|k| (k, "x".repeat(100))).collect();
    BBTreeMapFile::<u64, String>::create(&out, &map).unwrap();
}
```

Run it twice, as two processes, and compare the files:

```
cargo run --release -- a.bbt && cargo run --release -- b.bbt && cmp -l a.bbt b.bbt | wc -l
# expected: 0
# bbtree-mmap 4657a46 (macOS arm64): 64
```

The bytes that differ are in the root page, from offset 1544, one 8-byte word every 104 bytes (each slot of the old leaf's 100-byte values): `0xc7f400958`, `0xc7f4009c0`, `0xc7f400a28`, ..., pointers into the writing process's heap. On Linux x86_64 the same happens within one process, with two `create` calls and an allocation between them (160 bytes differ, words such as `0x73920f2cd968`).

## Notes

- `RootPage::raise` (`bbtree/src/tree/node/root_page.rs`) moves the root leaf to a fresh page and re-initializes the root page in place as an internal node. `NodePtr::initialize` writes the header and the inline-key and hash-tag columns, so the old leaf's value slots stay behind in bytes the internal node does not use. In the writer's tree, those slots hold `ArenaBytes` pointers.
- `FileWriter` copies the root page whole and rewrites only the edges of internal pages, so the stale pointers reach the file. Only the root page is affected: every other page is freshly allocated and zeroed.
- As far as I can tell, readers never read these bytes (they lie outside the internal node's layout), so this is about what the file holds, not about reading it back.
- For maps of plain types the stale bytes are copies of entry data, so those files come out the same every time, but they still hold bytes the entries alone do not determine. That is why `file_maps_depend_on_the_entries_alone` passes: it uses `u64` values.

Found by an automated property-based tester (ephyra); drafted with AI assistance.
<!-- owner: add in your own words whether you reproduced this yourself -->

