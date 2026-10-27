# bbz — bbtree harness

The seed harness for the bbtree target. It was ported from `case-studies/bbtree/properties`
(cargo tests written by hand, run for hours locally and under ASan without a finding) to the
ephyra-agent harness contract. It aims at what bbtree's own fuzz targets (`bbtree/fuzz`: the in-memory maps
against std's `BTreeMap`, libFuzzer-sized inputs) and Shuttle tests (small trees) do not reach.

`build.sh <src> <build>` builds `crate/` (Rust; depends on `<src>/bbtree` with feature `file`, `<src>/bbtree-mmap`,
`<src>/bbtree-store` by path, and on ephyra at `$EPHYRA/crates/ephyra`, default `/opt/ephyra`) with profile `fuzz`
(release + debug assertions + overflow checks, no LTO) and `--cfg fuzzing`, which compiles in bbtree's own
`check_invariants` / `check_prefix_lens`, into `<build>/bin/bbz`. It also tries `<build>/bin/bbz-asan`
(nightly, `-Zsanitizer=address`, installing the nightly toolchain if it is missing). If that fails it warns and goes on.
`BBZ_NO_ASAN=1` skips the ASan build.
`build.sh <src> <build> coverage` builds `bbz` with `-C instrument-coverage` (explicit `--target`, so build scripts
and proc macros stay uninstrumented) and writes `<build>/coverage.json` with the binary and the toolchain's
`llvm-profdata`/`llvm-cov` (it needs the `llvm-tools` component).

`run.sh <build> <seed> <secs> <out>` runs `bbz search`: a supervisor with `BBZ_WORKERS` (default 2) `bbz worker`
children. **Odd seeds run `bbz-asan`** when it exists (with `ASAN_OPTIONS=detect_leaks=0:abort_on_error=1` and a
90 s hang limit). Each worker draws properties by weight and runs a chunk of each through ephyra's `check`
(seeded, shrinking on, regressions off). Before every case it writes the case to `<out>/.inflight-<i>.json`. If a worker
dies (ASan report, SIGSEGV, abort, stack overflow), the supervisor records `crash__abort__<oracle>` with that case. If
the in-flight file is not touched for `BBZ_HANG_S` seconds (30, 90 for ASan, 600 on the coverage build, which runs 8-64x slower), it kills the worker, records
`crash__hang__<oracle>` and restarts it. Workers stop min(5 s, 5%) before the deadline and exit normally, so
instrumented builds flush their profiles. `<out>/stats.json` is rewritten every 2 s. It has `cases`,
`failures`, `crashes`, `hangs`, `worker_restarts`, `signatures`, `cases_<property>`, `ms_<property>`
(worker-milliseconds) and `label_<label>`. Workers' stderr goes to `<out>/worker-<i>.log`.

Findings are `findings/<sig>/<n>/{finding.json, case.json, repro.sh}`, at most 3 per signature. `case.json` holds the
property, signature, binary and the shrunk input (serde JSON). Within a chunk, the first failure fixes the signature:
while shrinking, failures of another signature are discarded, so the minimal case keeps it. `repro.sh <build>` runs
`bin/<binary> repro case.json`. That runs the case in a child process (`bbz run-case`) and exits 1 when it fails with
the same signature. For `crash__abort__*` the child dying counts, and for `crash__hang__*` running past `BBZ_HANG_S`.
It exits 0 when no attempt fails and 2 when it cannot decide. Threaded properties get 20 attempts.

Other commands: `bbz list` (properties, weights, chunk sizes), `bbz gen <property> <seed> <n>` (samples).

## Oracles (`crate/src/`)

Signatures are `<oracle>__<kind>__<area>`. A panic that escapes a property is `crash__panic__<oracle>`. Every
oracle compares against std's `BTreeMap`, or checks a contract sentence from bbtree's docs.

1. **filemap** (`filemap.rs`, 18 properties). A `FileWriter` → `FileMap` round trip for Auto's inline-key types
   (u64, i64, u128, i128, `[u8; 16|20|32]`), whole-compare keys (u8, u16, u32, `[u8; 7]`), and non-default node
   configs and strategies (`Configured<Auto | WithInlineKey, BigLeaves | TinyInternals | BigInternals>`,
   `WithInlineKey`). It checks that `write()` streams the same bytes as `to_buf()`, `byte_len`, `from(&map)` ==
   `from_ascending`, `open` accepts, len, first/last, `iter().len()`, every 64th stored key, and random get / get_many
   / double-ended walks over `range` and `iter`. Maps from 0 to 40k entries come from a small `MapSeed` (start, stride,
   len, extras).
2. **filemapdmg** (4). `FileMap::open` on damaged bytes: byte flips, word overwrites with offset-like values, copies,
   truncation (1–4 damages, placed near the header or at word alignment). Per its docs, a damaged map that passes `open`
   "may still give wrong answers, or panic, but never undefined behavior". So reads after `open` run under
   `catch_unwind`; only UB (ASan, odd seeds), hangs, or a panic in `open` itself (`filemapdmg__panic__open`) count.
   Labels: `opened`, `answers_changed`.
3. **mmap** (4). `BBTreeMapFile` for byte-string, text and u64 keys/values, written by `create` and by `write_at`
   (embedded after an offset in a larger file; the prefix must be untouched). Opened by `read`, `mmap`, `read_range` and
   `mmap().range()`, each compared with std (len, iter, rev, get, get_key_value, ranges).
4. **mmapdmg** (3). `BBTreeMapFile::read`, which checks the file, on damaged bytes. The contract is the same as
   filemapdmg. A text view of non-UTF-8 bytes panics by design, and is allowed. `mmapdmg__panic__read` is a panic in
   `read` itself.
5. **store** (4 configs: defaults, 64-byte internal pages, 8 KiB heap pages, 4 KiB leaves). A differential test of
   `BBTreeStore` against `BTreeMap<Vec<u8>, Vec<u8>>`: insert/remove/get/scan/scan_rev and double-ended `range`, keys
   up to past `max_key_len`, and values up to 20 KB (overflow pages). The allocator refuses pages from chosen points on
   (`Op::Budget`), and a refused insert must leave the store as it was (`store__mismatch__whole`). Pages out after the
   drop must be 0 (`store__leak__drop`). Freed pages are scribbled over. Label: `alloc_refused`.
6. **storethreads** (2 configs). 2–4 threads on one store, with self-describing values (key, version, length, fill),
   so torn, misplaced or freed records show. Scans must be ordered, in range, and each key once, and keys one thread
   alone wrote must hold its last write.
7. **syncmap** (u64 and String keys). `bbtree::BBTreeMap` under page refusals: refused `insert`, `insert_many`
   entries, `update_or_insert` and `get_or_insert` hand the entry back, run no closure and leave the map unchanged.
   Values count their drops (`syncmap__drop_count__values`), and pages and values after the drop must be 0. Labels:
   `refused`, `refused_in_batch`.
8. **bulkmap** (10 key/config combinations). The unsync map at up to 10^5 entries with removal-heavy phases
   (strided removes, pop runs, retain, partial extract_if, split_off + append, contiguous runs). Each phase is compared
   whole, followed by bbtree's `check_invariants` + `check_prefix_lens` (`bulkmap__invariant__check`). Labels:
   `peak_10k`, `peak_50k`, `removals_from_10k`.
9. **syncthreads** (4: u64/String × default/narrow). 2–6 threads on a `BBTreeMap` preloaded with up to 60k
   entries: inserts, removes (single and runs), `insert_many`, `update_or_insert`, get, `get_many`, scan/scan_rev and
   `range_cloned`. Values name their key. Keys written by one thread alone are checked after the join, followed by
   `check_invariants`.

## Known differences masked

- An inverted range (start past end, or equal and both excluded) on an **empty** map or store: std doesn't
  panic, because it checks the bounds only when it has a root. bbtree's maps match std. The store and the sync
  map panic even when empty. Their docs say "as std's `BTreeMap::range`", which is a cosmetic difference, so on an
  empty map either behaviour is accepted. On a non-empty one, the panic must match std's.
- Panics while reading a damaged file map after `open` or `read` accepted it: allowed by the docs. They're not counted
  separately.

## Throughput (local, Apple M-series, 2 workers; cases/s per worker)

| oracle | plain | ASan |
|---|---|---|
| filemapdmg | 7050 | 950 |
| filemap | 2240 | 360 |
| syncmap | 1600 | 280 |
| storethreads | 580 | 110 |
| store | 360 | 43 |
| mmapdmg | 100 | 32 |
| bulkmap | 90 | 17 |
| mmap | 52 | 10 |
| syncthreads | 17 | 6 |

A 120 s run gives about 200k cases on the plain build and 43k on ASan. A 60 s instrumented run covered 9.8k of
13.3k lines of bbtree's sources (bbtree, bbtree-store, bbtree-mmap).

## Self-tests (off by default; for checking the harness, never set in a search)

- `BBZ_SELFTEST_BREAK=1`: the filemap round trip reports `filemap__mismatch__get` for any map of 100 or more entries.
  A run must produce that finding, shrunk to exactly 100 entries, and its `repro.sh` exits 1 with the variable set
  and 0 without it.
- `BBZ_SELFTEST_ABORT=1`: the store properties `abort()` on a case with three or more removes. A run must produce
  `crash__abort__store` findings, and their `repro.sh` exits 1 with the variable set and 0 without it.
- Hang detection: `BBZ_HANG_S=3`, then `kill -STOP` a worker. The supervisor records `crash__hang__<oracle>` and
  restarts it.
- `BBZ_PANIC_LOG=1` prints panics to the worker log (they are silenced by default).
