# bbtree harness

Property tests for [bbtree](https://github.com/XiangpengHao/bbtree), aimed
at the parts its own fuzz targets (`bbtree/fuzz`, which cover the
in-memory maps against std's `BTreeMap`) don't reach. The checkout is
`bbtree/` at the repository root (gitignored); clone it there first:

```bash
git clone https://github.com/XiangpengHao/bbtree.git bbtree
```

| Test file | What it checks |
|---|---|
| `tests/file_map.rs` | `bbtree::file`: `FileWriter` → `FileMap` round trips against `BTreeMap` (get, get_many, first/last, double-ended walks over `iter` and `range`), for inline-key and whole-compare key types and for non-default `NodeConfig`s and strategies; `FileMap::open` on damaged bytes (flips, word overwrites, copies, truncation) refuses them or reads without UB or a hang |
| `tests/mmap.rs` | `bbtree-mmap`: `BBTreeMapFile` written by `create` and `write_at` (embedded after other data), opened by `read`, `mmap`, `read_range` and `mmap().range()`, against `BTreeMap` for byte-string, text and integer keys and values; `read` on damaged files |
| `tests/store.rs` | `bbtree-store`: a differential test against `BTreeMap<Vec<u8>, Vec<u8>>` (insert, remove, get, scan, scan_rev, double-ended `range`), values up to overflow pages, at four node configs, with an allocator that refuses pages at random moments (a refused insert leaves the store as it was) and counts pages out after the drop |
| `tests/store_threads.rs` | `bbtree-store` shared by 2 to 4 threads: self-describing values, so torn or freed records show; scans in order, within range, each key once; keys only one thread wrote hold its last write |
| `tests/sync_map_alloc.rs` | `bbtree::BBTreeMap` under page refusals: refused `insert`, `insert_many` entries, `update_or_insert` and `get_or_insert` leave the map unchanged, run no closure, and hand the entry back; values count their drops |

The damaged-input and threaded properties are meant for AddressSanitizer;
panics there are allowed (the documented contract is "wrong answers or a
panic, never UB"). A watchdog in `src/lib.rs` aborts with the input if one
case runs past its deadline.

```bash
cargo test                                         # 200-300 cases per property
EPHYRA_TESTS=5000 cargo test --test file_map       # more
RUSTFLAGS="-Zsanitizer=address" CARGO_TARGET_DIR=target/asan \
  EPHYRA_TESTS=100000000 EPHYRA_BUDGET_MS=1200000 \
  cargo +nightly test --target aarch64-apple-darwin --test file_map damaged
RUSTFLAGS="-C instrument-coverage" CARGO_TARGET_DIR=target/cov \
  EPHYRA_COVERAGE_GUIDED=1 EPHYRA_COVERAGE_FILTER=bbtree \
  cargo test --features coverage --test file_map damaged_u64_u64 -- --nocapture
```
