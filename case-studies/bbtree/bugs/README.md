# Bugs: bbtree

The bugs the case studies found in bbtree. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [BBTreeMap::get_or_insert: a panicking closure during a leaf split aborts the process (debug) or leaks the reserved pages (release); closed by documenting that user code must not panic](bbtree-get-or-insert-closure-panic-in-split-abo/) | declined | [link](https://github.com/XiangpengHao/bbtree/issues/25) |
| [BBTreeMap: dropping the map while a key's or value's Drop panics leaks every page on its free lists; closed by documenting that user code must not panic](bbtree-latched-destroy-skips-free-lists/) | declined | [link](https://github.com/XiangpengHao/bbtree/issues/29) |
| [BBTreeMapFile::create writes the writer's heap addresses into the file's root page, so the same entries give different files](bbtree-root-raise-leaves-stale-arena-pointers-i/) | open | [link](https://github.com/XiangpengHao/bbtree/issues/31) |
| [WalWriter::open reports Corrupted on a healthy WAL when another writer fills the last file during the scan](bbtree-wal-open-skips-appends-to-a-file-filled-/) | open | [link](https://github.com/XiangpengHao/bbtree/issues/23) |
