# rustc optimizer harness

Target: rustc trunk (stage1, `rust.debug-assertions=true`, CI LLVM 23 with assertions, LLVM + cranelift backends).

## Files

- `build.sh <src> <build> [coverage]`: rsyncs `<src>` into `<build>/src` (keeps the `.git` file so bootstrap can
  pick the CI LLVM artifact; mtimes preserved, so rebuilds are incremental), fetches `library/backtrace` and a
  sparse `compiler-rt` of the pinned `src/llvm-project` commit (no `git submodule`), writes `<build>/bootstrap.toml`
  and runs `x.py build --stage 1 library`. `<build>/rustc` links to the stage1 compiler. ~10 min cold.
  `coverage`: the same with `RUSTFLAGS_BOOTSTRAP=-Cinstrument-coverage`, i.e. the stage0 (beta) compiler instruments
  the stage1 compiler crates (`librustc_driver`, cranelift backend); std is not instrumented. `coverage.json` points at
  ci-llvm's `llvm-profdata`/`llvm-cov` (23.1.4; the beta's LLVM is 23.1.1, profile formats are compatible).
  Coverage paths are under `<build>/src/...`; the controller maps them to `work/src` by suffix.
- `run.sh` -> `run.py`: the search loop. `gen.py`: the program generator. `oracle.py`: configs, compile/run,
  crash and diff classification, and the per-finding checker (`oracle.py repro <build>`).

## Generator (`gen.py <seed>`)

Random **safe** Rust programs, valid by construction (100% compile at baseline over 400 seeds): structs, enums
(unit/tuple/named variants, `#[repr(int)]` with explicit discriminants, niches via `Option`), arrays, tuples,
`Option`, `Box`, `Vec`, closures (`move`), iterator chains (rev/skip/take/step_by/filter + fold/count/max/min/position),
`loop` with labeled `break value`, `for`/`while`, `match` on ints (ranges), enums (guards, bindings) and tuples of
enums, `&mut` parameters, reborrows, `mem::swap/replace`, slices, a trait with default methods used statically
(generic), dynamically (`&dyn`, `Box<dyn>`), wrapping/checked/saturating/overflowing integer ops on all int widths
incl. i128/u128, f32/f64 (`+ - * /`, sqrt, mul_add, floor..., casts), `std::hint::black_box`, inline attributes.
No recursion (functions call only earlier ones), bounded loops, cost budget on calls inside loops.
main prints the `Debug` of every function result.

35% of programs are *panicky*: compiled with `-Coverflow-checks=on` and they also use plain `+ - * / % << >>`,
`abs`, `pow`, `sum/product`, unchecked indexing and `unwrap`; main wraps each call in `catch_unwind` (silent
hook) and prints `Some(result)` / `None`. Panics are deterministic, so every config must agree.

## Oracles

1. **Crash/ICE** (`ice__<kind>__<area>`): any config where rustc exits 101/by signal or prints an ICE, a panic,
   an LLVM assertion, `LLVM ERROR`, or `-Zvalidate-mir`'s "broken MIR"; or a compile that takes > 90 s.
   kind = panic / debug_assert / bug / llvm_assert / llvm_error / broken_mir / signal / hang; area = compiler
   crate + file of the panic location (e.g. `codegen_ssa_analyze`), the LLVM source file, or the MIR pass.
2. **Multi-config differential** (`miscompile__<kind>__<area>`): baseline `-Copt-level=0 -Zmir-opt-level=0`
   vs 4 configs sampled per case from `oracle.CONFIGS` (mir-opt 1-4 at -O0, `-Zmir-move-elimination`,
   -O1/2/3/s/z, -O3 with mir-opt 0 or 4, aggressive `-Zinline-mir`, cranelift at -O0 and -O3). Compared: stdout
   and exit status (kind = stdout / exit / signal / timeout). On a mismatch all configs are run and the area says which
   family is guilty: `mir_opt` (some -O0 + MIR-opt config differs, LLVM not needed), `mir_move_elim` (only
   with -Zmir-move-elimination), `llvm` (-O3 with MIR opts off differs), `mir_llvm` (MIR opts + LLVM),
   `mir_inline`, `cranelift`, `mir_cranelift`. Representative config in finding.json/case.json.
   `diff__compile_error__<family>`: a config rejects a program the baseline compiles.
3. **Option toggles** (steering round 1), same comparison against the baseline, blamed only when no config of
   the families above differs: `debuginfo` (`-Cdebuginfo=2 -Zverify-llvm-ir` at -O3, at -O0 with mir-opt 3, at
   -O2 with move elimination; `-Zmir-strip-debuginfo` + inline-mir), `llvm_target_cpu` (`-Ctarget-cpu=x86-64-v3/v4`,
   this host has AVX-512), `cgu` (`-Ccodegen-units=1 -Zshare-generics`; fat/thin LTO cost ~10 s per compile, not used),
   and `mir_pass_toggle`: every case gets one extra config `toggle` with a random `-Zmir-enable-passes` set
   (1-4 optional passes off from `oracle.TOGGLE_OFF`, 0-2 default-off passes on from `TOGGLE_ON`, random opt
   levels, sometimes move elimination / debuginfo). The exact flags are stored as `cfg_flags` in case.json and
   in finding.json's message.
4. **CTFE vs run time** (steering round 2, `consteval__mismatch__<int|int128|flt>`): 30% of non-panicky gen.py
   programs (~20% of cases, `gen.CONST_SHARE`) are *const programs*: every function is a `const fn` (only the
   const-callable subset: no closures/iterators/traits/Box/Vec/`for`/aggregate `==`; `Ord::min`, `unwrap_or`,
   `Option::filter/map` are spelled as `if`/`match`). main evaluates every call twice, as `const K<k>: T = f(..)`
   (the const evaluator, rustc_const_eval + rustc_apfloat for floats) and as `f(black_box(..))` at run time, and
   prints `c<k>:<cls> {:?}` and `r<k>:<cls> {:?}`. A pair that differs in the baseline binary is a finding (mode
   `consteval`, repro compiles/runs the baseline only); value class = what the value contains (any float, else any
   128-bit int, else int). Differences between configs go through oracle 2 as usual.
   Both modes also use newer integer methods (isqrt, checked_ilog*, abs_diff, midpoint, leading/trailing_ones,
   unbounded shifts, wrapping/saturating/checked pow, byte round trips, cast_signed, *_add_signed/unsigned,
   div_ceil, next_multiple_of, ...) and float `round`, `round_ties_even`, `fract`, `signum`, `recip`, `%`, `clamp`
   (all exact or correctly rounded in IEEE 754, so CTFE and hardware must agree; `mul_add` is const too).

5. **Destructors** (steering round 3): 45% of non-const gen.py programs are *droppy*: they define `struct D(i64)`
   whose `Drop` folds its payload into a thread-local hash and decrements a live count (`D::new` and `Clone`
   increment it; `forget_d` forgets and decrements). `D` appears in structs, enums, tuples, arrays, `Option`, `Box`,
   params and returns; `move_stmt` moves non-Copy locals out whole, conditionally (`if c { drop(x) }`, `if c { x }
   else { .. }`: drop flags), partially (struct/tuple field, `*box`, by-value `match` binding one variant field,
   edition-2021 disjoint closure captures), re-initialises maybe-moved locals, moves them into FnOnce closures, and
   `if let Some(w) = x`; payload stores right before a drop are observable (DSE). main runs every call in its own
   block and then prints `drop: <hash> <n>`. Panicky programs unwind through live `D`s (cleanup blocks).
   `drop__<leak|double>__base`: n != 0 at the baseline (mode `dropcount`; Reference, Destructors: each value is
   dropped once when its scope ends). A config whose output differs from the baseline only in `drop:` lines gets
   the area suffix `_drop` (e.g. `miscompile__stdout__mir_opt_drop`).

## rustlantis share

8% of cases (`HARNESS_RUSTLANTIS`; at 15% the MoveElimination difference below alone was 1.4% of cases) are custom-MIR programs from rustlantis (cbeuw/rustlantis, pinned commit, built
by build.sh into `<build>/rustlantis`, with `rustlantis-noref.patch`: raw pointers instead of references, because its
reference tracking makes ~95% of programs Stacked/Tree Borrows UB; patched programs pass Miri `-Zmiri-tree-borrows`).
  The patch also makes `addr_of!` pick only initialized places: under the RFC 3943 semantics of `-Zmir-move-elimination`
  (rust-lang/rfcs#3943: "Any use of the local that doesn't initialize it, even taking its address, is UB if the local
  is unallocated") a local only gets an allocation when it is written directly, so taking the address of a
  never-written local and writing through the pointer is UB there (Miri doesn't model it) and MoveElimination may
  overlap that local with another one (cluster `miscompile__timeout__mir_move_elim_custom_mir`).
- ~35-45% hit the known ICE rust-lang/rust#149191 (`codegen_ssa/src/mir/analyze.rs:159`, "Post-projection ...
  should be non-Ref") even at -Zmir-opt-level=0: base is compiled first and such cases are skipped
  (`rl_known_ice`), also for other configs (`KNOWN_ICE_CUSTOM_MIR` in run.py). Other ICEs are reported normally.
- Behaviour differences are only reported after Miri (nightly under `work/ref`, Tree Borrows) runs the program
  without UB and prints the baseline's output (`rl_miri_clean/ub/differs/unavailable`); area gets a `_custom_mir` suffix
  (`miscompile__stdout__mir_move_elim_custom_mir`), so they don't merge with gen.py clusters.
- `-Zmir-move-elimination` opts into other MIR semantics (RFC 3943: a local is allocated by its first direct
  write and freed by a bare `move`, so taking the address of an uninitialised local, or using a local after
  `Move(_n)`, is UB). rustlantis does both all the time, and nightly Miri doesn't check the new rules, so ~20% of its
  programs differed under the flag (the steering-round-1 failures below were all this). `rlfix.py` rewrites each program
  so it is valid under both semantics: it adds a scalar write at function entry for every address-taken local and
  turns bare `Move(_n)` into copies. If it can't, the program runs without the move-elimination configs
  (`rl_no_move_elim`). A side effect is that #149191 no longer fires on the rewritten programs.

All configs share `--edition 2021 -Awarnings -Coverflow-checks=off|on -Cdebug-assertions=off -Zub-checks=no
-Zvalidate-mir`: overflow checks/debug assertions/UB checks default differently per opt level, so they are pinned.

## Masked / avoided differences (documented nondeterminism)

- NaN sign/payload (RFC 3514): floats are only printed with `{:?}`, which prints every NaN as `NaN`.
- Libm precision: no transcendental functions; only `+ - * /`, sqrt, mul_add, floor/ceil/trunc/abs/min/max/copysign.
- Addresses, HashMap order, type sizes, `type_name`, `Location`: never printed. Panic messages (stderr, include a TID)
  are not compared.
- cranelift: panic=abort by default (Readme), so panicky programs never go to `cl0`/`cl3`.
No output is normalised otherwise. Counters in stats.json: `gen_invalid` (baseline rejects the program),
`base_bad_run` (baseline binary crashes/times out), `flaky` (a finding whose repro didn't fail right away; marked
`UNSTABLE` in its dir), `panicky`, `compiles`.

## Findings

`findings/<sig>/<n>/`: `prog.rs`, `case.json` (mode, config, flags), `oracle.py` (copy), `repro.sh <build>`:
exit 1 reproduces, 0 not, 2 undecided (baseline doesn't compile/run). Each finding is re-run once at record time.

## Throughput

One run.sh (HARNESS_JOBS=4 threads, 5 compiles/case): ~1.1 cases/s alone, ~0.7 cases/s with 4 run.sh in
parallel on 16 cores. Coverage build: ~0.25 cases/s (instrumented rustc + profile writes).

## Own gate runs (2026-10-21)

- 4 x 600 s (seeds 11-14): 1778 cases, 2 failures (0.1%): `ice__bug__cranelift_isle_umin_i128` (cl3:
  cranelift at -Copt-level=3 emits `umin.i128`, x64 lowering missing; real, not yet triaged).
- 4 x 900 s (seeds 21-24): 2598 cases, 0 failures, 2 gen_invalid (fixed since), 0 base_bad_run, 0 flaky.
- Coverage build, 1 x 240 s with LLVM_PROFILE_FILE=%4m: 40 cases; llvm-cov export over the 3 objects: 1856 compiler
  files, 127k of 402k lines covered.

## Ideas not done yet

Miri-gated unsafe/raw-pointer programs (rustlantis needs its aliasing tracker fixed), `-Zmir-opt-bisect-limit`
localisation in triage, larger aggregates for MoveElimination, debuginfo configs (-g), LTO/codegen-units.

Steering round 1 local runs (2 x 300-400 s, machine shared with the search, 0.1-0.25 cases/s each, 15% rustlantis):
219 cases, 3 failures, all rustlantis programs that Miri (Tree Borrows) runs cleanly and that print something else
with `-Zmir-move-elimination`; `-Zmir-enable-passes=-MoveElimination` brings back the baseline output.
