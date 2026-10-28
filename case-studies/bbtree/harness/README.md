# bbz — bbtree harness

**State (2026-10-28).** This is the harness as the agent run of 2026-10-25..28 left it: the seed
below plus 19 steering rounds (WAL, stateful models of the whole map/set API, leaf annexes,
concurrent linearizability checkers, schedule perturbation, page-size and type matrices, file
maps over new types) and the triage and owner fixes. The run found four bugs: #23 (WAL), #25 and
#29 (panicking user code), #31 (heap addresses in file maps). Since 74be068 (bbtree 0.1.6+) user
code must not panic: after a panic in `Ord`, `InlineKey`, `Clone`, `Drop` or a closure the map stays
memory-safe but its contents are unspecified (#25 and #29 were closed on that ground). The panic
oracles (`syncpanic`, `upanic`, the panics in `syncvacant`) still check leaks and post-panic contents;
for a run against current bbtree, narrow them to memory safety (ASan, no UB) first.

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
It also tries `<build>/bin/bbz-chaos`, the **schedule-perturbation build** (`BBZ_NO_CHAOS=1` skips it): a copy of
`<src>` in `<build>/chaos-src` whose `Interleaving::switch_point` and `Interleaving::wait`
(`bbtree/src/tree/interleaving.rs`: Shuttle's switch points, before every access to an atomic latched-tree threads
share, and every round of waiting for a latch) call the harness's hooks in `crate/src/chaos.rs`, built with
`--cfg bbz_chaos`. Each thread draws a mode every few hundred to few thousand points: quiet, short spins, long spins,
yields, rare stalls (a holder that lost its CPU); waiters yield now and then. Nothing else in the copy changes.
Its workers run only the threaded properties (repro attempts > 1). A finding there names `bbz-chaos` in `case.json`,
so its `repro.sh` replays on the same build; a race it shows is one a plain build can hit, less often.
`build.sh <src> <build> coverage` builds `bbz` with `-C instrument-coverage` (explicit `--target`, so build scripts
and proc macros stay uninstrumented) and writes `<build>/coverage.json` with the binary and the toolchain's
`llvm-profdata`/`llvm-cov` (it needs the `llvm-tools` component).

`run.sh <build> <seed> <secs> <out>` runs `bbz search`: a supervisor with `BBZ_WORKERS` (default 2) `bbz worker`
children. **Odd seeds run `bbz-asan`** when it exists (with `ASAN_OPTIONS=detect_leaks=0:abort_on_error=1` and a
90 s hang limit); **seeds ≡ 2 (mod 4) run `bbz-chaos`** when it exists (not under coverage). Each worker draws properties by weight and runs a chunk of each through ephyra's `check`
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
   Bulk-loaded keys no thread writes must be found by every get, handed out by every scan and range over them (from
   either end), and hold their value after the join (`storethreads__missing__<get|scan|range|final>`; store.rs: a
   scan is "as a map's scan" not a snapshot, and a map's scan visits "an entry present for the whole scan").
7. **syncmap** (u64 and String keys). `bbtree::BBTreeMap` under page refusals: refused `insert`, `insert_many`
   entries, `update_or_insert` and `get_or_insert` hand the entry back, run no closure and leave the map unchanged.
   Values count their drops (`syncmap__drop_count__values`), and pages and values after the drop must be 0. Labels:
   `refused`, `refused_in_batch`.
7b. **syncpanic** (`syncpanic.rs`; u64, String, a user key `PK` under `WithInlineKey`, narrow pages; plus
   `syncpanic_store`). Bombs armed on the calling thread panic at the n-th call of a closure (`update`,
   `update_or_insert(_many)`, `get_or_insert`, `remove_if`, `read(_many)`, `scan(_rev)`, store scans) or of the keys'
   and values' `Ord`/`Clone`/`Drop`, alone or while 1–3 threads work their own interleaved keys. Closure panics are
   in the contract (design/09 "Panics": the latch unlatches on unwind, a half-changed value stays): the model is
   exact afterwards, no later call hangs, `check_invariants` passes, nothing leaks. Teardown (`clear`, drop, the
   rest of an `into_iter`) with a panicking `Drop` must still drop every entry once, keys before values in key
   order, and free every page (`Tree::destroy`, `DyingWalk` docs): `syncpanic__{drop_order,leak}__<call>`.
   Other `Ord`/`Clone`/`Drop` panics (D48) taint the map: only double drops (live marks), aborts and hangs count.
   Batch closure panics (D49: outside the contract) read the batch's keys back. **syncvacant** is the same with
   `get_or_insert`'s closure panicking on a new entry, which runs inside a split after its pages are reserved:
   kept in its own family so that its aborts (`crash__abort__syncvacant`) don't hide others.
8. **bulkmap** (10 key/config combinations). The unsync map at up to 10^5 entries with removal-heavy phases
   (strided removes, pop runs, retain, partial extract_if, split_off + append, contiguous runs). Each phase is compared
   whole, followed by bbtree's `check_invariants` + `check_prefix_lens` (`bulkmap__invariant__check`). Labels:
   `peak_10k`, `peak_50k`, `removals_from_10k`.
9. **syncthreads** (8: u64/String × default/narrow, plus 4 `_shrink`). 2–6 threads on a `BBTreeMap` preloaded with up to 60k
   entries: inserts, removes (single and runs), `insert_many`, `update_or_insert`, get, `get_many`, scan/scan_rev and
   `range_cloned`. Values name their key. Keys written by one thread alone are checked after the join, followed by
   `check_invariants`. The 4 `_shrink` properties (u64, String default/narrow, String WithInlineKey) map the lowest
   key numbers outside the tree-wide shared prefix (D50) in steps (`t0`, `tenant/08`, `tenant/7/a16`,
   `tenant/7/key/!24` before `tenant/7/key/0000032`; for u64, 2 and 3 shared bytes instead of 5), never preloaded:
   a thread's insert shrinks the prefix while others scan, get and remove (issue #9's race). Ops draw from the lowest
   40 numbers 1 time in 20, and 1 space in 7 is tiny (2–64), so the tree empties and the prefix restarts.
   All 8 also check the documented visibility rule (`map/mod.rs` `scan`: "an entry present for the whole scan is
   visited exactly once"; `range_cloned` "hands out an entry other threads insert ahead of it"): preloaded keys no
   thread writes are found by every get/`get_many_copied`, visited by every scan/scan_rev and `range_cloned` over
   them, and hold their preload value after the join (`syncthreads__missing__<get|get_many|scan|range|final>`).
10. **wal** (5: `MAX_APPENDS` 1, 2, 3, 7, 10000). `bbtree_store::wal::WalWriter` on storage in memory that keeps the
   storage contract of `design/bbtree-store/01-wal.md` (compare-and-append, whole or nothing, `MAX_APPENDS` a file),
   loses requests and responses (`Op::Faults`) and reads (`Op::ReadFaults`), lets another writer append before a
   chosen storage call (`Op::Rival`: a claim and/or appends, as a writer still alive or a new one would), and damages
   bytes between operations (flips, truncation, garbage past the end). The harness's own decoder of the documented
   layout reads the files as the ground truth. Checks: after a writer's operation the WAL still decodes
   (`wal__format__decode`) and no file takes more than `MAX_APPENDS` (`wal__contract__max_appends`); an open replays
   every record before its claim, in order, at its addresses (`wal__mismatch__replay`), writes exactly one claim
   (`wal__writes__open`), fails with `Corrupted` at the first failed check when one fails (`wal__corrupt__accepted`,
   `wal__corrupt__offset`) and only then (`wal__corrupt__false_alarm`); an `Ok` append's addresses hold its records
   (`wal__mismatch__address`), `TailMoved` wrote nothing and `Storage` at most once (`wal__writes__append`); a writer
   fenced by a later claim writes nothing (`wal__fence__append`); `read_value` gives an insert's value and
   `Corrupted` for a removal (`wal__mismatch__read_value`). `BBZ_WAL_DEBUG=1` adds the write log and the files'
   records to a failure's message. Labels: `open_<error>`, `append_<error>`, `rival`, `rival_ran`, `damaged`,
   `multi_file`.

11. **umodel** (13 unsync + 5 sync; values `Val<P>` behind the `UVal` trait so typemx can swap them: u64, i128, `[u8; 20]`, String, `Vec<u8>`, `Box<[u8]>` under Auto, WithInlineKey,
   narrow and wide configs; values padded 0, 3 or 40 words). A stateful model of two unsync maps and two sets against
   std, over the API the other oracles never call: the entry API (every method of `Entry`, `VacantEntry`,
   `OccupiedEntry`, `first_entry`/`last_entry`), `iter_mut`/`values_mut`/`range_mut`, `into_iter`/`into_keys`/
   `into_values` dropped part way, `Clone` (with a value clone that panics part way: no leak, original unchanged),
   `Extend`, `get_many`/`insert_many`, `==`/`cmp`/`Index`; on sets, insert/remove/replace/take, set algebra walked and
   built by the operators, `is_subset`/`is_superset`/`is_disjoint`, split_off/append/retain/extract_if/pop/into_iter.
   Walks check `size_hint` at every step (exact for `ExactSizeIterator`s), finish by `next`, `fold`, `rfold` or
   `count`, and stay finished; `Extremes` compares `last`/`min`/`max`/`nth`/`nth_back`/`count` and iterators cloned
   part way. Key modes give short keys, keys behind a shared prefix, keys of 255+ bytes (saturated hash tags) and a
   mix whose shared prefix comes and goes. After every op: whole contents, `check_invariants`/`check_prefix_lens`,
   live values = model sizes (`umodel__leak__<op>`); after the drop, pages and values 0. The `umodel_sync_*`
   properties run the same op lists on `bbtree::BBTreeMap` from one thread (`get_mut`, `iter_mut`, `into_iter`,
   `clear`, `read`/`update`, `remove_if`, `update_or_insert`, `get_or_insert`, scans with early breaks,
   `range_cloned`, the batch calls). Signatures `umodel__<mismatch|walk|size_hint|leak|invariant>__<op>`.
   The `umodel_tag*` / `umodel_sync_tag*` properties (4 unsync + 2 sync) use keys with a tag `Ord` ignores
   (`Tagged`: a string, `TagU64`: a u64; tag = the number of the op that made the key), whose `InlineKey` is the
   inner type's (the docs' `UserName` example), under `WithInlineKey` (inline keys, separator keys in leaf heads,
   hash tags) and Auto (plain). Every result and the whole contents then also compare by `Debug`, tags included:
   which of two equal keys the map keeps is documented (`insert`: "the map keeps its key and drops the given one";
   set `insert`: "keeps its element"; `replace` puts the given one in (also every n-th element at once, so
   separator keys get replaced); `append`: "`self` keeps its key"), as std's.
   Labels: `entry_api`, `clone_panicked`, `set_algebra_nonempty`, `maps_1k`.
12. **annex** (4 single-thread: u64, String, String narrow, `Vec<u8>` WithInlineKey with 260-byte keys; 3 threaded).
   `bbtree::BBTreeMap` with a leaf annex (`WithLeafAnnex<S, Probe>`) driven through `update_or_insert_with_leaf`
   and `remove_with_leaf`, the API bbtree-store's record heap is built on. The probe annex has an id and a token count
   (an accepting vacant closure adds one, a removal takes one back, a split hands half to the new leaf, a merge takes the
   other's, the teardown releases the rest); a registry mirrors every annex, so an annex copied, dropped unreleased with
   tokens, released twice, or seen with other tokens than it was left with shows (`annex__stale|lost|double|fresh__*`).
   In the closures the leaf's keys must be ascending, hold the key (or not, vacant), and be a run of the map's keys with
   none missing, the vacant key fitting in it (`annex__keys__*`). Closures refuse (vacant, occupied, both) and a page
   budget refuses nodes: a refused insert changes nothing and hands the entry back; a vacant closure that accepted means
   the insert happened (`annex__closure_ran__refused`). `clear` must release every annex, `into_iter` drained part way
   then dropped and the drop must release all (`annex__leak__*`, `annex__tokens__*`), pages and values 0 after.
   The `annex_threads_*` properties: 2-4 threads on interleaved keys (neighbours belong to different threads) with a
   spin while the leaf is held: no two closures hold one annex at once, no split/merge/release touches a held annex
   (`annex__exclusive__*`), the held leaf's keys don't change (`annex__moved__keys`) and its keys of this thread are
   exactly the ones it wrote (`annex__keys__threads`). Labels: `annex_split`, `annex_merged`, `annex_released_tokens`,
   `refused`.

`BBZ_ONLY=<family|prefix>` makes workers draw only that oracle's properties, or those whose name starts with it (for trying one oracle; never
set in a search).

13. **synclin** (6: u64 / String keys, Auto, narrow internals, WithInlineKey, with and without a shrinking shared prefix;
   values of 3, 4 or 8 words). `bbtree::BBTreeMap` with 2–6 threads, checked while they run. Each thread owns keys
   interleaved with every other thread's (`(i * threads + t) * 2 + 1`), so leaves are shared, but only it changes them:
   every answer about them has one right value from its own model, at every step (`synclin__mismatch__<op>` for
   insert, remove, remove_entry, remove_if, update, get_or_insert, upsert, batch, get, contains_key, get_many, scan,
   range, is_empty). Batches (`insert_many`, `update_or_insert_many`, `get_many_copied`/`_cloned`, `read_many`) have 1,
   4, 16 or 70 lanes (past 64 the executor polls slots). Up to 16 hot keys (even numbers, never removed) count
   additions by `update_or_insert`, `update_or_insert_many` and `get_or_insert` + `update` ("threads that add to the
   same key's value this way miss none of each other's additions"): after the join each holds the sum
   (`synclin__lost__add`), and while running a thread never reads a count below one it read or made before
   (`synclin__stale__<op>`) nor misses a hot key it has seen (`synclin__missing__<op>`). Values carry words derived
   from key and payload, so a torn optimistic copy or another thread's write on an owned key shows
   (`synclin__torn__<op>`). Also scan / range_cloned order (front and back joined, `synclin__order__*`), whole
   contents after the join (`synclin__final__contents`) and `check_invariants`. Allowed and not checked: range_cloned
   is not fused; within one batch read lanes may read a repeated hot key in any order.
   `synclin_u64_w300` / `synclin_string_shrink_w130` use values of 2.4 KB / 1 KB: leaves of 12 entries spread over
   several pages, which `get_copied` copies whole while other threads write.
14. **ukey** (18, `ukey.rs`). User `InlineKey` impls under `WithInlineKey`: the node-plan rows no std key reaches.
   `UK<W, F>` is a byte string (bytes = encoding, `Ord` = theirs) whose impl sets `WIDTH` 2/4/8, and by `F` the hash
   tag (an FNV byte, not std's length), `cmp_from` (really starts at `equal_len`, so too large an `equal_len` answers
   wrong) and its own `cmp_prefix` (else the trait's default). Keys: groups behind a u32, keys that tie when
   zero-padded and differ only in length, a 70-byte shared prefix (past the 64 a search checks), 255+ bytes. The impl
   records broken call premises from the docs ("given that the first `equal_len` bytes ... are equal", "Called only
   if HAS_CMP_FROM / HAS_HASH_TAG", prefix "at most 64 bytes") as `ukey__contract__<cmp_from|cmp_from_called|hash_tag|cmp_prefix_len>`.
   The keys drive the umodel (unsync, narrow, sync), annex (single and threaded), synclin and bulkmap runners; their
   failures come back as `ukey__<kind>__<area>`.
15. **typemx** (18 unsync + 6 sync). The umodel model and ops (`umodel::tm`, `umodel::tm_sync`) over key and value
   types no other property uses, so node layouts other than the usual ones are built: keys `u8`, `i16`, `u32`, `char`,
   `bool`, `()` (every insert collides), `(u8, u16)`, `Option<u64>`, a 64-byte-aligned key `K64`, `[u8; 3]` and
   `[u8; 300]` (also under WithInlineKey, which gives any array inline keys), `[u8; 64]`; values aligned to 16, 64
   and 128 bytes (`Val<P, A16|A64|A128>`: zero-sized alignment fields) and of 1 KB / 2.4 KB (`Val<130>`,
   `Val<300>`: nodes held at the 12-key minimum, several pages each); sets of 1- and 2-byte keys reach the
   1023-key cap (`typemx_i16_wide`: 4 KiB leaves). The sync map needs inline keys (it panics "a BBTreeMap's keys
   need inline keys" otherwise, by design), so its rows use Auto's integers/arrays and WithInlineKey arrays.
   Signatures are umodel's with the prefix `typemx__`.
   Both umodel and typemx run, on every `Compare` op, the std traits the walks implement on their own instead of
   taking the default (`last`/`min`/`max` of iter_mut, values_mut, into_iter, into_keys, into_values, `len`,
   `partial_cmp`, set `is_empty`) and `Debug` of maps, sets and part-consumed consuming walks, against std
   (`<oracle>__mismatch__<partial_cmp|debug|iter_mut_glue|into_iter_glue>`; `debug` is only a cosmetic difference).

15. **storelin** (4 configs: defaults, 64-byte internal pages, 8 KiB heap pages, 4 KiB leaves; `storelin.rs`).
   `BBTreeStore` with 2–6 threads, checked while they run. Each thread owns keys interleaved with every other
   thread's (key number `i * slots + t`), so a leaf's records lie in heap chains of leaves other threads hold
   (splits move no record; compaction, orphans and pulls happen under all of them), yet only the owner changes
   them: insert's "whether it replaced a value", remove's "whether the store held it", get, `get_into` ("as
   `Read::read_to_end` does", buffer prefilled, unchanged on `None`), `contains_key`, and scan / scan_rev (with
   early breaks) / `range` from both ends over stub or key bounds, inclusive or exclusive, must give exactly the
   thread's model for its own keys (`storelin__mismatch__<op>`); every entry handed out must be whole, in range and
   in order (`storelin__torn|order__<op>`). Inserts and removes in runs of up to 120 own keys empty and fill leaves.
   Pages are refused to one thread at a time inside one insert (a thread-local budget of 0–3 pages): a refused
   insert returns `Alloc` and the key keeps its old value ("The store holds what it held then",
   `storelin__mismatch__refused`). Up to 8 hot keys, preloaded and never removed, take self-describing values
   from all threads (`storelin__missing|torn__hot`). Key shapes: short, mixed with some near `max_key_len`, long,
   behind a 30-byte shared prefix; values 12 B to 20 KB (overflow pages). After the join the store is the union
   of the models (`storelin__mismatch__final`), and after the drop no page is out (`storelin__leak__drop`).
   Label: `refused`.

16. **sametree** (22, `sametree.rs`). Documented same-tree relations of the unsync map and set, checked with
   bbtree's own `check_same_nodes` (`--cfg fuzzing`; length, shape, keys, head keys, prefix lengths), which no other
   property calls: `insert_many` "make[s] the same tree, node by node, keys and prefix lengths included, as individual
   inserts" (`design/12-batch-api.md`), results "exactly as separate calls to `insert` do"; `extend` (16 at a time
   through `insert_many`, always under `--cfg fuzzing`, the partial tail one by one; exact or no size hint) "Inserts each
   entry in turn, as `insert` does"; `Clone` copies "node by node, so that the copy has the same shape"
   (`tree/clone.rs`). Batches of 0 to 128 entries (past the executor's 64 pinned lanes), keys near ones already used
   (several splits in one leaf in one batch), repeated keys, keys leaving the shared prefix; `get_many` /
   `contains_many` of 2 to 128 keys against std. Between them removals (runs), pops, retain, split_off, clear, clones.
   Key/value/config rows from umodel/typemx (u64, narrow, i128 wide, `[u8; 20]`, String, String/`Vec<u8>` inline,
   `Tagged`/`TagU64` (results and contents also by `Debug`: which equal key is kept), u8, i16 wide, `()`, `[u8; 3|300]`
   inline, K64) and ukey's `UK` (failures stay `sametree__*`, broken premises `ukey__contract__*`).
   `sametree_bomb_*`: keys whose `Ord` panics at an armed call (`Op::Arm`; `BombKey` plain, `BombStr` with the bytes'
   `InlineKey`, only `Ord` panics). D48: after such a panic the map need not be valid but stays sound, so the ops go
   on unchecked and only a double drop / double free counts (`sametree__double_drop|double_free__panic`; ASan on odd
   seeds). Signatures `sametree__<nodes|contents|mismatch|invariant>__<insert_many|set_insert_many|extend|set_extend|clone|set_clone|get_many|contains_many|...>`,
   `sametree__leak__<values|pages>`. Labels: `big_batch`, `batch_on_nonempty`, `map_1k`, `ord_panicked`.

17. **shlin** (6: u64 / String keys, Auto, narrow internals, WithInlineKey, with and without a shrinking shared
   prefix; values of 2, 3, 4 or 130 words; `shlin.rs`). `bbtree::BBTreeMap` with 2–6 threads that all insert,
   remove (single calls and runs of up to 400 keys), `remove_entry`, `remove_if`, `update`, `update_or_insert`,
   `get_or_insert` and batch (`insert_many` / `update_or_insert_many`, 1/4/16/70 lanes, keys may repeat) the
   **same** keys: a few hot keys far apart in a tree of up to 3000 (12000 with never-written keys between them,
   stride 2-4), or a whole tree of 1-4 keys that empties and fills. Every call is stamped by a SeqCst clock before
   and after; every write's value carries a unique token. After the join each key's history is checked alone for
   linearizability against a one-value register (locality; Wing & Gong search memoised on the threads' positions
   and the value; `lin_giveup` label past 200k states). Lanes of a write batch are separate calls in input order
   ("as separate calls to `insert` would", "not atomic"); a read batch's lanes are reads in any order (first lane
   of a key checked). A scan / `range_cloned` (both directions, early stops) is per key a read at some point of the
   walk: visited with the value of that moment ("each while no thread changes it"), or not visited only if absent
   at some point ("an entry present for the whole scan is visited exactly once"). Failures
   `shlin__nonlinear__<area>`: the read class whose removal makes the history linearizable (`get_copied`,
   `scan`, `range_cloned_rev`, `get_many_copied`, ...), else `reads`, `writes_batch` or `writes`. Also
   `shlin__torn__<op>` (a value not whole or another key's), `shlin__order|phantom|missing__<walk>` (out of order,
   a key nobody wrote, a never-written key not handed out), `shlin__refused__<op>`, `shlin__final__contents`,
   `shlin__invariant__check`, `shlin__panic__thread`. Labels: `contested`, `tiny_tree`, `lin_giveup`.

18. **upanic** (7: integer keys under `Auto` and narrow internals, byte-string keys with the std impl's tags and
   `cmp_from` under `WithInlineKey` (only `Ord` panics) and narrow, byte-string keys with the trait's default
   `InlineKey` whose `inline_key_at` panics too, under `WithInlineKey`, narrow and wide; `upanic.rs`).
   `unsync::BBTreeMap` / `BBTreeSet` (preloaded up to 12000) whose user code panics at an armed call: a key's `Ord`
   or `InlineKey`, a key's or value's `Drop`, the `extract_if` / `retain` closure, the iterator `extend` pulls.
   sametree checks only soundness after a panic; these check the states the unsync rustdoc promises: `insert` /
   `remove` / `remove_entry` / `entry` / set `insert|remove|replace|take` unchanged after an `Ord` panic ("every call
   to it happens before the first change"), unchanged or done after an `InlineKey` or `Drop` one; `append` keeps
   its own entries and a key-order prefix of the other map's, the other left empty; `split_off` unchanged after
   `Ord`; `extract_if` / `retain`: the element whose closure panicked stays (values the closure changed stay
   changed), those before it processed, those after untouched, every entry gone yielded; `extend`: the entries
   before the iterator's panic in, less at most the 16 pulled ahead; `insert_many`: each key old or batch-given;
   `clear` / the map's drop / `IntoIter`'s drop: the drop log is every entry left, ascending, key before value,
   even when one `Drop` panics, and the map empty. After every op: `check_invariants` (skipped after an
   `InlineKey` panic, which upstream says may leave inline keys off their prefix; the map is then rebuilt) and
   `check_prefix_lens`, `len` against the walk, and an id ledger: alive == held by the map and set, none dropped
   twice (after a `Drop` panic outside a teardown, what the unwind leaked is forgotten). At the end no value alive
   and no page out. Signatures `upanic__<state|mismatch|invariant|leak|double_drop|drop_order|len|panic>__<op>`
   (`state`: a bomb went off; `mismatch`: none did). Label `bomb_fired`.

19. **mmapmx** (18, `mmapmx.rs`). `bbtree-mmap`'s `BBTreeMapFile` and `bbtree::file`'s `FileWriter` / `FileMap` over
   the plain key types, value types and input paths the mmap / filemap oracles never take: keys `[u8; 3|16|20|32|70]`,
   `i8`, `u16`, `i128`, the docs' `UserId` (`plain_disk_type!(UserId, WithInlineKey)`) and byte-array user keys
   `Uk<N>W<w>` with their own `InlineKey` (width 2/4/8, FNV hash tags, `cmp_from`, own or default `cmp_prefix`;
   broken call premises are `mmapmx__contract__<hash_tag|cmp_from|cmp_from_called|cmp_prefix_len>`); values `u8`,
   `i16`, `u128`, `f32`/`f64` of any bits (compared by bytes), `[u8; 0|5|33]`, `String`, `Vec<u8>`. Keys come from
   a per-case shared prefix (0-72 bytes) plus a short tail, so they collide, share prefixes and tie in inline keys.
   Written from std's map, the unsync map, the sync map by its owner (`iter_mut`), owned entries, `&mut` entries,
   `Cow`/plain inputs; opened by `read`, `mmap` (small or huge pages), `read_range`, `mmap().range()`: len,
   is_empty, iter both ways, get, get_key_value, `get_many` of 1-4 keys, ranges walked from both ends, against std
   (`mmapmx__mismatch__<op>`). `write_at` returns the range it wrote, ending the file, prefix kept
   (`mmapmx__range__write_at`), and its bytes are `create`'s (`mmapmx__mismatch__write_at_bytes`). Two `create`s of
   the same entries, the arena moved between them, write the same bytes (`mmapmx__nondeterministic__create`;
   tree/file.rs: slots hold spans or "zeros, not stale pointers"). 1 case in 6 hands over entries out of order
   (a swap, a repeat, a reversed tail): `create` and `write_at` must fail with `InvalidInput` and leave the file
   as it was (`mmapmx__accepted|touched__<create|write_at>_disordered`), `FileWriter::from_ascending` must fail with
   `NotAscending` at the first entry not above the one before (`mmapmx__accepted|index__from_ascending`). For plain
   values also `FileWriter::from(&unsync)`, `from(&mut sync)` and `from_ascending` write the same bytes
   (`mmapmx__mismatch__from_<unsync|sync>`), and `FileMap` `keys`/`values`/`Debug`, ranges cloned part way and
   `rfold` match std. Labels: `disordered`, `mmapmx_1k`.
20. **synclinr / shlinr** (6 + 4; `synclin.rs` `run_refuse`, `shlin.rs` `run_refuse`). The synclin and shlin
   threads unchanged, on a map whose allocator (`synclin::Flaky`) refuses pages to each thread at its own rate
   (0, 1-16, 16-200, 200-1023 or 1024 of 1024 allocations, always or in windows of `on` of every `on + off` ops,
   a per-thread xorshift): one thread's splits and root growth fail while others' go through, and the pages
   removals free go to whichever thread asks first (`tree/latched/pages.rs`). A refused `insert`,
   `update_or_insert`, `get_or_insert` or batch lane must hand back its exact entry (`*__refused_entry__<op>`), run no
   closure (`*__closure_ran__<op>`; in a batch: closure runs == lanes that found their key), and leave the map
   unchanged (`InsertError`: "the map is unchanged, and the entry comes back"). synclinr re-reads the own key at once
   (`synclinr__refused_changed__<op>`) and its model holds it to that from then on; shlinr puts a refusal into the
   key's history as a read that found the key absent ("a node the insert makes": a held key makes none), so a
   refusal of a key present throughout shows as `shlinr__nonlinear__refused_<op>`. Every other signature as the
   base oracle's, prefixed `synclinr__` / `shlinr__`; after the drop no page is out (`*__leak__pages`). Labels:
   `refused`, `refused_100`, `refused_in_batch`, `refused_existing` (a refused insert of a key the thread holds;
   allowed, never seen).

21. **zkey** (27, `zkey.rs`). The exact oracles (umodel unsync/sync, sametree, bulkmap, syncthreads, synclin, shlin,
   synclinr, shlinr) over byte-string / `String` keys of a shape no other property makes: the trie preorder of
   {0x00, 0x01, 'a', 0x7f} up to 11 bytes (rank 0 is the empty key, then "\0", "\0\0", ...), so keys are prefixes
   of each other and differ only in trailing zero bytes (what a zero-padded inline word or a window past a shared
   prefix can't tell apart). Byte order is rank order (5.6M keys, more than synclin's 1.5M hot numbers). umodel modes
   add the shared prefix, 300-byte heads, zero runs of 7/8 bytes and prefixes of the shared prefix; under shrink the
   threaded properties' lowest numbers are the prefixes of "tenant/7/" and that prefix as a whole key. A process-wide
   switch turns the shape on for one case; checks are unchanged, signatures are `z<oracle>__<kind>__<area>`.

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

- `BBZ_SELFTEST_BREAK=1`: the filemap round trip reports `filemap__mismatch__get` for any map of 100 or more entries;
  shlin's `get_copied` of a key the thread removed reports the key's preloaded value (`shlin__nonlinear__get_copied`).
  A run must produce that finding, shrunk to exactly 100 entries, and its `repro.sh` exits 1 with the variable set
  and 0 without it.
- `BBZ_SELFTEST_ABORT=1`: the store properties `abort()` on a case with three or more removes. A run must produce
  `crash__abort__store` findings, and their `repro.sh` exits 1 with the variable set and 0 without it.
- Hang detection: `BBZ_HANG_S=3`, then `kill -STOP` a worker. The supervisor records `crash__hang__<oracle>` and
  restarts it.
- `BBZ_PANIC_LOG=1` prints panics to the worker log (they are silenced by default).
