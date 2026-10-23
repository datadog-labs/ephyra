# binaryen harness

Differential and crash fuzzing of `wasm-opt` (binaryen trunk), using **wasmtime 50** as the executor.
binaryen's own `fuzz_opt.py` never runs wasmtime, so this path is new.

## Layout

- `build.sh <src> <build> [coverage]`: builds `wasm-opt`/`wasm-dis` (Release + assertions, static,
  `BUILD_LLVM_DWARF=ON`) into `<build>/binaryen`, copies them to `<build>/bin`, builds the seed corpus
  `<build>/seeds`, and builds the Rust driver (`<build>/cargo`, then `bin/bynfuzz` and `bin/bynexec`).
  With `coverage`, wasm-opt is compiled with `-fprofile-instr-generate -fcoverage-mapping`, and
  `coverage.json` names `bin/wasm-opt` plus llvm-profdata-18/llvm-cov-18 (clang 18). The Rust driver is
  never instrumented.
- `run.sh <build> <seed> <secs> <out>`: execs `bynfuzz`, one single-threaded search loop. The environment
  (`LLVM_PROFILE_FILE`) is inherited by every wasm-opt process.
- `make-seeds.sh`: `--initial-fuzz` corpus. These are the target's `test/lit/**/*.wast` files that have one
  module and parse/validate under the harness feature set, minus `scripts/test/fuzzing.py`'s
  `unfuzzable` list and `*.fail.*`. About 566 files, each ≤ 32 KiB.
- `driver/src/exec.rs` (`bynexec m.wasm`): runs a module in wasmtime (embedded crate, cranelift, DRC GC)
  the way `scripts/fuzz_shell.js` / `src/tools/execution-results.h` do. It provides the
  `fuzzing-support` imports (log-*, throw, table-get/set, call-export(-catch), call-ref(-catch), sleep,
  log-branch, the `wasmtag`/`jstag` tags, and synthesized externref globals with the same payload
  formula), then calls every export in order with default arguments. Exit 0: output on stdout. Exit 3:
  host limit (stack overflow, fuel, GC heap, allocation size). Exit 4: wasmtime cannot compile or link
  the module.
- `driver/src/fuzz.rs` (`bynfuzz`): the search loop. `stats.json` also has `fail:<signature>` counts.
- `driver/src/smith.rs`: wasm-smith module generation (oracle binary-read, below).

## Per case

1. Random bytes (256 B to 32 KiB) go through
   `wasm-opt -ttf <features> [--initial-fuzz=<seed>] [--closed-world] [--denan] --translate-to-exnref`
   to make `a.wasm`. Seeds are used in 50% of cases, closed world in 30%, `--denan` in 80%. Unseeded cases also
   disable each optional feature (simd, stack-switching, multimemory, memory64, tail-call, threads,
   wide-arithmetic, custom-page-sizes, EH) with 15% probability. Seeded cases keep the full base set,
   because seeds use those features. Before that fix, 2.7% of cases were bogus `crash__*__ttf` findings.
   In 20% of cases a stack-switching program from `driver/src/ss.rs` (see below) is made first; half of
   those are used as they are (`a.wasm`, no -ttf), the other half are the `--initial-fuzz` content that
   -ttf builds on (instead of a lit seed).
2. `bynexec a.wasm` produces the expected output. This step runs only for de-NaN'd modules. Oracle
   **interp** then compares it with binaryen's interpreter on the same module.
3. `wasm-opt a.wasm <features> <pipeline>` produces `b.wasm`. The pipeline is a random list of pass groups
   from fuzz_opt's `opt_choices` plus passes fuzz_opt never picks (merge-similar-functions,
   avoid-reinterprets, propagate-globals-globally, reorder-globals, untee, ssa-nomerge,
   duplicate-import-elimination). It also gets an optional `--roundtrip`, -ol/-sl, `--converge`,
   `-fimfs`, `-pii` and `--expect-later-toolchain-opts`. Closed-world-only passes are picked only with
   `--closed-world`, and that flag is used only when the module was generated with it.
4. `bynexec b.wasm` must print the same output. When it does, 30% of cases also run oracle **interp** on
   `b.wasm`, and 25% of all cases run oracle **roundtrip** on `b.wasm`.

## Oracles and signatures

- **crash** (`crash__<kind>__<pass>`): wasm-opt must not fail on a valid module, either when generating
  or when optimizing. `kind` is `assert`, `signal`, `hang` (30 s), `validation`, `fatal` or `exit`.
  `pass` is the last `running pass:` printed by a rerun with `BINARYEN_PASS_DEBUG=1`, which also
  validates after every pass. It is `ttf` for generator crashes and `io` when the crash happens before any
  pass runs.
  Generator failures that print no `Fatal` (plain "can't fuzz this") are skipped (`skip_gen_error`).
- **exec** (`exec__mismatch__<pass>`): wasmtime output before vs after the pipeline. `<pass>` comes from a
  bisect. The pipeline is expanded to the passes it ran (from `BINARYEN_PASS_DEBUG=1` output, top level
  only), keeping every global flag and the -O level's `--optimize-level`/`--shrink-level`. The search
  then finds the shortest failing prefix. If the expanded list does not reproduce, the first flag of
  the pipeline is used.
  - `exec__reject__<first-flag>`: wasmtime refuses `b.wasm` even though it accepted `a.wasm`.
  - `exec__enginecrash__<first-flag>`: wasmtime crashed on `b.wasm`. This is probably a wasmtime bug, so
    triage it as a reference bug.
  - `exec__traplost__<lowering-pass>`: a lowering pass (below) removed a trap the original module hit,
    e.g. memory64-lowering wrapping an out-of-bounds i64 address to 32 bits. Kept apart from
    `exec__mismatch__<lowering-pass>` so other wrong results of the same pass stay visible.
- **lowering** (same crash/exec oracles; the pass is the signature's area): 30% of pipelines (60% in the
  numeric profile) get one group from `LOWER_GROUPS`, semantics-preserving passes that neither fuzz_opt
  nor `PASS_GROUPS` run: memory64-lowering (table64-lowering is its alias),
  multi-memory-lowering-with-bounds-checks (only without threads/memory64/custom-page-sizes), signext-lowering, llvm-nontrapping-fptoint-lowering, dealign,
  optimize-for-js, coalesce-locals-learning, limit-segments, intrinsic-lowering, alignment-lowering
  (needs `--disable-simd`: "unhandled unaligned load"), and in the numeric profile outlining, asyncify,
  flatten (alone, + dfo, rereloop, local-cse, simplify-locals-notee-nostructure) and -O4. Half the time the
  group goes last (where toolchains run lowerings), else anywhere; 20% of those pipelines are the group
  alone. No `--converge` with a lowering group (asyncify would add its globals twice; fuzz_opt avoids repeated flattening: "exponential overhead").
  - **Numeric profile**: 15% of unseeded cases disable EH, GC, reference types, multivalue, tail calls and
    stack switching (half also memory64, threads and custom-page-sizes, so that multi-memory-lowering, which asserts that all memories are configured alike, applies). Flatten fails on BrOn and EH blocks and cannot
    spill non-nullable refs/tuples, asyncify rejects references and tail calls ("not yet supported"), and
    outlining emits invalid code on 80% of full-feature modules, so these run only here.
  - `--asyncify` appends `asyncify_*` exports; bynexec skips them (and leaves them out of call-export's
    index space), and the interp oracle is skipped on such optimized modules.
  - llvm-nontrapping-fptoint-lowering gets the crash oracle only: out of range it returns INT_MIN/0, by
    design ("otherwise it's UB so any value will do", LLVM's fptosi semantics), not the saturated value
    (`exec_skipped_crash_only`).
  - memory64-lowering wraps every address to i32 (a 32-bit memory cannot hold addresses >= 4 GiB; such
    programs "cannot be correctly lowered", #8316), so a lost trap whose cause, per binaryen's interpreter
    on a.wasm, is a load/store through a pointer >= 2^32 (`final|highest > memory: <ptr>`) is skipped
    (`skip_memory64_lowering_wrapped_address`). Other lost traps of the pass are still reported.
  - Fatal errors that state a pass limitation (`MultiMemoryLowering: ...`, asyncify's "not yet supported" and
    "Please specify which of the multiple memories to use" (wasm-smith modules have several),
    "Unsupported instruction for Flatten") are skipped (`skip_lowering_limitation`).
  - Left out: llvm-memory-copy-fill-lowering ("should only be run on modules with no passive
    segments"), i64-to-i32-lowering and legalize-js-interface (they change the import/export ABI),
    optimize-added-constants (needs `--low-memory-unused`).

- **interp** (`interp__mismatch__<class>`, `interp__mismatch__ss-<class>` for cases with an ss.rs
  program): `wasm-opt m.wasm <features> --fuzz-exec-before` (binaryen's interpreter, which Precompute,
  ctor-eval and `--fuzz-exec` trust) must print what wasmtime prints. Both are canonicalized by
  `driver/src/interp.rs`: trap kinds and exception payloads masked, NaNs as `nan`, refs as in
  `printValue`, floats compared by value (binaryen prints f32 via JS numToString of the double),
  i64 log arguments split into two i32s as execution-results.h does (`BYNEXEC_SPLIT_I64`), `contref`
  log arguments (unreadable in wasmtime) as wildcards. Skipped when binaryen hits a `[host limit ...]`
  (its call depth limit is 200) or a cont crosses the host boundary. `<class>` is the kind of the first
  differing line: trap, exception, length, simd, branch, ref, float, int. repro.sh runs
  `bynfuzz --interp-check <build> m.wasm <features>`.
- **interp, pre-exnref** (`interp__mismatch__pre-exnref-<class>`): in half of the de-NaN'd EH cases the
  module is generated again without `--translate-to-exnref` (legacy try/catch/delegate/rethrow, which
  wasmtime cannot run). Binaryen's interpreter on that module must match wasmtime on its translation,
  so a difference is a bug in TranslateToExnref (whose only check so far was validation) or in the
  interpreter's legacy EH. repro.sh: `bynfuzz --interp-check <build> a0.wasm --translate <features>`.
  The translation is done in its own process on the module read back from `a0.wasm` (the generation
  step runs it in-process on the generator's in-memory module), and a failure there is reported as
  `crash__<kind>__translate-to-exnref-reload`. First run: about 0.3% of cases, all one bug
  (`br_on_cast_fail` over a `try` whose body is exact-typed: the new `try_table` gets the exact type and
  the cast type is not adjusted, "br_on_cast* target type must be a subtype of its input type").
- **roundtrip**: `wasm-opt b.wasm <features> --name-types -S` (as fuzz_opt's RoundtripText; the type names
  avoid a known collision) then parsing that text back must work (`crash__<kind>__text-print`,
  `crash__<kind>__text-parse`), the result must load in wasmtime (`roundtrip__reject__text`) and, for
  de-NaN'd modules, run the same as `b.wasm` (`roundtrip__mismatch__<class>`).

- **text-read** (`driver/src/wat.rs`): the roundtrip oracle only ever parses binaryen's own printing. Here a
  second implementation, wasmprinter (wasm-tools 0.261), prints the module in the standard text format,
  folded or flat (50%), with or without synthesized `$#func0`-style names for unnamed items (30%), so the
  parser sees spec syntax binaryen never prints: flat instruction sequences, index references, `(;N;)`
  comments, table initializer expressions, `(@custom ...)` annotations, `try_table` catch lists, labels on
  every block/if. Runs on `a.wasm` in 30% of cases (not ss.rs ones) and on `b.wasm` in 15% of the passing
  cases that skip the roundtrip oracle. `wasm-opt m.wat <features> --strip-debug --strip-producers
  --strip-target-features` must succeed (`crash__<kind>__text-read-<area>`, area as for binary-read, or the
  words of the parser's `error:` message) and give the module that reading `m.wasm` the same way gives:
  binaries equal, else `wasm-opt -S` of both binaries equal without custom-section comments (names and
  custom sections are stripped first, binaryen does not keep `@custom` annotations). A difference is
  `text-read__mismatch__<class>` when the text-read module also runs differently in wasmtime (de-NaN'd
  modules), else `text-read__diff__<keyword of the first differing line>`. `bynfuzz --wat <m.wasm> <fold
  0|1> <names 0|1>` prints the text. First run (90 s), two parser bugs:
  - A table initializer is parsed only when it starts with `(` (`parsers.h` `table`, `peekLParen`); the
    spec's `(table tabletype expr)` takes a flat `ref.null nofunc` too ("expected end of table declaration").
    About 20% of flat prints hit it, so 98% of modules with a table initializer are printed folded
    (`text_read_throttled_table_init`).
  - A labeled folded `if` whose condition sets a non-nullable local (`(if $l (ref.is_null (local.tee $x
    ...)) ...)` then `local.get $x` after it) fails validation ("non-nullable local's sets must dominate
    gets"); without the label it reads fine.

- **binary-read** (`crash__<kind>__binary-read-<area>`): 25% of the cases without an ss.rs program start
  from a wasm-smith module (`driver/src/smith.rs`, wasm-smith 0.261, the wasm-tools release wasmtime 50
  uses). wasm-smith is configured from the case's features (random_features, including the numeric
  profile): no imports, everything exported, `canonicalize_nans`, and `ensure_termination` (a fuel global
  decremented at each loop head and call, a deterministic trap). Modules that wasmparser does not validate
  under the case's features are dropped (`smith_invalid`). `wasm-opt smith.wasm <features>` must then load
  and validate it: the spec says the module is valid, so a parse exception, assertion or validation error
  is a reader (wasm-binary.cpp / IRBuilder) bug. `<area>` is the source file of an assertion, or the first
  words of the parse exception or validator message (stable, no values). Skipped (`READ_LIMITS`,
  `skip_binary_read_limitation`): limitations binaryen states, "Recursion groups of size zero not
  supported" (a TODO in wasm-binary.cpp), and MVP-only element segment rules when reference types are off.
  Generator avoidances: wide arithmetic only with multivalue (binaryen holds its two results in a tuple),
  and tail calls stay on when GC is on (wasm-smith emits `return_call_ref` with tail calls off, which
  wasmparser accepts under GC).
  - Binaryen's reader drops declared element segments, so later segment indices shift (an open bug:
    "invalid element segment index", or silently the wrong segment). It hit about 1.5% of wasm-smith
    modules, so 90% of modules with a declared segment are regenerated
    (`smith_throttled_declared_elem`).
  - Half of these modules become `a.wasm` as they are (`smith_standalone`). First,
    `exec__mismatch__binary-read`: the module as binaryen read and wrote it back (no passes) must run in
    wasmtime like the original. Then every other oracle (exec, lowering, interp with signature prefix
    `smith-`, roundtrip) runs on them. wasm-smith's `canonicalize_nans` does not cover
    `f64.promote_f32`/`f32.demote_f64` (and the SIMD forms), whose NaN results are nondeterministic, so
    modules with those get the crash oracles only (first run: an interp false positive through
    promote + reinterpret). Since they log nothing,
    bynexec also prints a hash of every exported memory after the exports ran (`[memhash "<name>"]
    <bytes> <fnv64>`, name Debug-escaped; `BYNEXEC_MEMHASH=1`, set
    in exec repro.sh too; the interp oracle drops those lines, binaryen's interpreter prints none).
  - The other half are the `--initial-fuzz` content -ttf builds on (instead of a lit seed), so -ttf's
    logging and its feature mix are combined with wasm-smith's code shapes.

### Stack-switching programs (ss.rs)

`-ttf` makes `cont.new` but never `resume`, `suspend` or `switch`, so binaryen's stack-switching runtime
had never run (and V8, fuzz_opt's only VM, cannot run it). ss.rs writes .wat modules of coroutines driven
by scheduler exports: `resume` and `resume_throw` with handlers for four tags (one of which abandons
the coroutine), a first run through `cont.bind`, two interleaved coroutines, suspends from callees and
from nested sub-schedulers that handle only some tags (the rest are forwarded to the outer handler),
exceptions thrown into a suspended coroutine and caught there or outside, re-resuming a consumed
continuation (trap). A quarter of the modules also have a pair of symmetric coroutines that `switch`
to each other; binaryen's interpreter cannot run `switch` (ModuleRunner::visitStackSwitch is
unimplemented and `--fuzz-exec` aborts with an uncaught NonconstantException), so those cases skip
the interp oracle (`interp_skip_switch`) and get only the wasmtime ones. All values are i32, loops are bounded, there is no recursion, every suspend is
handled within its export. `bynfuzz --ss <n>` prints one. `ss_invalid` counts modules that do not parse
(must stay 0; the first three are saved as `<out>/ss-invalid-*.wat`).

Each finding directory has `a.wasm` (or `input.dat` plus `initial.wasm` for generator crashes),
`b.wasm`, `before.out`/`after.out`, `cmd.txt`, `stderr.txt` and `repro.sh <build>`. The script exits
1 when the failure reproduces, 0 when it does not, and 2 when it cannot decide (for example a host limit).

## Masked differences (each documented by the target or the spec)

- **NaN bits**: "WebAssembly NaN signs are nondeterministic" (`src/passes/Precompute.cpp:24-27`), and the
  spec's NaN propagation rules agree. f32/f64 NaNs print as `nan`. A v128 half that is an f64 NaN prints
  `nan64`, and an f32-NaN lane prints `nan32`. NaN bits still leak through reinterpret and
  extract_lane (first gate run: `f64.max(nan, x)` then `i8x16.extract_lane_u` gave 255 in wasmtime
  and 0 when folded by Precompute). So, like fuzz_opt (`NANS=False` → `--denan`, and "with NaNs we
  can't compare the output"), the comparison runs only on modules generated with `--denan`. The other
  20% get the crash oracle only. Counter: `exec_skipped_nans`.
- **Trap kind**: every trap prints `[trap]` (fuzz_opt maps all of them to one exception, contract 6).
  Traps and exceptions are still told apart, because optimizations must not turn a catchable throw into
  a trap.
- **Reference identity**: refs print as `null`, `function`, `object`, `i31(N)`, `externref(N)` or
  `jserror`. This matches `printValue` in execution-results.h, where function indices and types may
  change.
- **Host limits** (stack overflow, fuel exhaustion at 200M units, GC heap OOM, allocation too large):
  the whole comparison is skipped, as with binaryen's `ExecutionResults::ignore`. Counters:
  `skip_a_host_limit` and `skip_b_host_limit`.
- **Continuation refs at the host boundary**: wasmtime 50's embedding API panics on them
  (`values.rs:865`, "embedding API for `(ref cont)`", wasmtime#10248). Exports with cont
  params or results are not called (`[skip: cont at the host boundary]`), and `log-contref` prints
  `contref`. Both runs get the same treatment.

## Excluded

- Features wasmtime 50 lacks: strings, fp16, custom-descriptors, multibyte, compact-imports,
  relaxed/acquire-release atomics and shared-everything. Relaxed SIMD is excluded too, because its
  results are implementation-defined and Precompute may fold them differently. Legacy EH is translated
  to exnref.
- Semantics-changing flags: -tnh, -iit, --fast-math, --low-memory-unused (ttf programs touch low
  memory) and -uim. Also flatten-based groups and -O4, which fuzz_opt avoids with EH and with
  multivalue plus reference types.
- `BYNFUZZ_EXTRA_OPTS` (env) appends flags to every pipeline. It is for self-tests only: with `-tnh`,
  6 mismatches in 77 cases showed that the exec oracle and repro.sh work.

## Throughput

- Idle machine: about 22-24 cases/s per process (bynexec takes about 7 ms per module, -ttf about 7 ms,
  -O3 about 10 ms). With 4 processes in parallel, about 16 cases/s each.
- Coverage build: about 9 cases/s. Each instrumented wasm-opt process writes a ~20 MB profile, so a
  `%m` (merging) `LLVM_PROFILE_FILE` pattern is much better than `%p`.
- First 4 × 240 s gate run (before `--denan`): 9854 cases and 6 failures (0.06%). The failures were
  `crash__validation__translate-to-exnref` (a -ttf module that `--translate-to-exnref` turns invalid:
  "block with value must not have last element that is none") and `exec__mismatch__precompute`
  (NaN-bit false positives, now handled by `--denan`).
- Final self-gate (4 × 240 s normal, plus 2 × 240 s coverage at the same time): 11,980 cases, about 12 cases/s
  each, 3 failures (0.025%, all `crash__validation__translate-to-exnref`). Coverage runs: about 6.4
  cases/s, 0 failures, 52% region coverage of binaryen's sources from the merged profile.
