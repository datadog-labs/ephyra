# ocfuzz: differential + crash harness for the OCaml native compiler

Target: OCaml trunk (`work/src`), native backends (closure and flambda1) against the
bytecode compiler of the same tree.

## Layout

- `build.sh <src> <build> [coverage]` builds two compilers from `<src>` in parallel:
  `<build>/closure/inst` (default configure) and `<build>/flambda/inst`
  (`--enable-flambda --enable-flambda-invariants`), then the Rust driver
  `<build>/bin/ocfuzz` (`ocfuzz/`). Incremental: the tree is rsynced into
  `<build>/<cfg>/src` and `make` reruns. With `coverage`, `cov/build-cov.sh`
  builds the same two compilers with `lambda/`, `middle_end/`, `asmcomp/`
  instrumented by bisect_ppx and writes `<build>/coverage.json`.
- `run.sh <build> <seed> <seconds> <out>` = `ocfuzz search`. 3 worker threads
  (`OCFUZZ_JOBS`), each running whole cases; a case compiles its 7 configs in
  parallel.
- `ocfuzz gen --seed S --dir D` writes the program of case seed S (the `seed` in
  `finding.json`); `ocfuzz check --build B --dir D` evaluates a case directory
  with all configs (handy for triage).
- `tools/linereduce.py <file> <cmd>`: greedy line-deletion reducer.

## Generated programs

Three modules: `h.ml` (fixed prelude: record/variant types, exceptions,
printers, total helpers), `a.ml` (generated module-level state + 2-6 functions,
some `[@inline]`/`[@inline never]`), `main.ml` (6-14 test functions `tN`, each run
under `H.run`, which prints a section header `==tN` and catches exceptions).
Modules are compiled together without `-opaque`, so the native configs inline
across modules through `.cmx`.

Typed generation (int, bool, float, int32/int64/nativeint, string, option, pairs,
lists, arrays incl. flat float arrays, refs, records with mutable fields, an
all-float record, a variant with constant/args/inline-record constructors,
`Atomic.t` on int/bool/int ref, `int->int`-like closures). Statements: lets,
observations (print), mutation, calls to A's and local functions (sometimes
`[@inlined never]`, `match ... with exception`), if/for/while (bounded), `match`
with guards / or-patterns / nested patterns (sometimes non-exhaustive ->
Match_failure), try-with, conditional raise, risky ops (`/`, `mod`, `Int64.div`,
`a.(i)`, `List.hd`, `Option.get`) alone in their own `let`, local functions,
bounded local recursion, Atomic get/set/exchange/compare_and_set/fetch_and_add,
lazy, `[@tail_mod_cons]` local list builders (recursive call under `::`,
`if`, `let`, `match`), recursive value definitions (mutually recursive closures,
a cyclic list observed through `H.take`, `lazy` and tuples referring to them).
Generic recursive definitions (`rec_group`, 2/3 of the let-rec statements and
at A's top level in 35% of programs) aim at `lambda/value_rec_compiler.ml`:
1-4 bindings of kinds function, cyclic record (`H.knot`, incl. `{H.knot0 with
...}`), cyclic variant (`H.kv`, inline record), cyclic list, float record,
lazy (direct or `let t = lazy .. in t`), function/float/int arrays, int
constant; each right-hand side under 0-3 wrappers that `Value_rec_check`
accepts (lets of pure values, aliases of recursive names, irrefutable and
always-matching refutable array/tuple/record/lazy/int/char/string patterns,
local modules, exceptions, refs, local `let rec` functions, `ignore`
sequences) and sometimes a nested `let rec` of the same kind. Recursive
functions are guarded (`x <= 0 || x > 10`, calls with `x - 1`), lazies force
only earlier lazies; observed through `H.ksum`/`H.kvsum`/`H.take`/calls.
A may also contain a functor applied twice, first-class modules unpacked at a
dynamically chosen instance, and a class with an inheriting subclass
(`super#`, `method!`, coercion). Matches scrutinise ints (incl. sparse
constants), variants, polymorphic variants (`H.pv`), options, lists, pairs,
strings (a shared pool of literals with common prefixes around the 8-byte word
boundary, for `asmcomp/strmatch.ml`), int arrays, boxed ints and float
constants. Pure expressions: arithmetic incl. division by literal constants, shifts,
`Int.popcount/leading_zeros/trailing_zeros`, float ops incl. fma/round/min_num,
float<->boxed-int conversions, bits_of_float/float_of_bits, unsigned ops,
polymorphic compare on structured values, `Sys.opaque_identity`, pure `match`,
`List.map/filter/fold_left` with lambdas, `Array.init/map/fold_left`.

Soundness rules (each from the manual/stdlib docs, see `ocfuzz/src/gen.rs` header):
evaluation order of arguments/tuple/record fields is unspecified
(`manual/src/refman/expr.etex`), so only pure, non-raising expressions appear in
those positions; everything effectful or raising is let-bound in sequence. Shift
counts are masked (`stdlib.mli`: unspecified if m < 0 or m > Sys.int_size, Int32
etc. similarly); float->int conversions are clamped (`stdlib.mli`: unspecified for
nan/out of range); nan is printed as `nan` (sign/payload unspecified); `==` is
never used on immutable values (`compare_and_set` only on immediates or refs);
no `Float.copy_sign`/`sign_bit` on nan, no polymorphic min/max on floats
(unspecified with nan); recursion and loops are bounded (no stack overflow).

## Oracles

1. **diff** (`diff__{value,exn}__<configs>`): stdout and exit status of
   the bytecode program (ocamlc, run under `ocamlrund`, the debug runtime) must
   equal those of each native config. Contract: `manual/src/cmds/native.etex`
   "Except on those points, the two compilers should generate code that behave
   identically." (listed exceptions: signals, FMA on non-amd64, unreferenced
   unit initialisation, stack overflow — none generated). `kind` = `exn` when the
   first differing test section involves an exception, else `value`; the area is
   the set of native configs that disagree, as short tags joined by `_`:
   `c` closure (default), `ci` closure_inline (`-inline 200 -dcmm-invariants -g`),
   `cc` closure_compact (`-compact -no-float-const-prop -no-insn-sched
   -function-sections`: code size / scheduling toggles), replaced by `cl`
   closure_linscan (the same plus `-linscan`) on 1 case in 32 (`LINSCAN_EVERY`,
   by case seed: see Known differences),
   `o3` flambda `-O3`, `oc` flambda `-Oclassic -g`, `ox` flambda `-O3
   -unbox-closures -rounds 4 -inline-max-depth 4 -remove-unused-arguments
   -runtime-variant d`. E.g. `diff__value__c_ci_oc` = closure and classic-mode
   flambda wrong, -O3 flambda right. One diff finding per case.
2. **crash** (`crash__compile__<build>_<area>`, `crash__run__<build>_<area>`,
   `hang__{compile,run}__<build>`): the bytecode compiler accepted the program,
   so every native compile must succeed (area = the `>> Fatal error:` word or
   the compiler source file of an `Assert_failure`, e.g.
   `crash__compile__closure_selection_size_expr`); ocamlc itself crashing gives
   `crash__compile__byte_<area>`; a binary killed by a signal or printing a
   runtime assertion is `crash__run__<build>_<area>` (area = runtime source
   file of the assertion, `fatal_error`, or `sigN`, e.g. `crash__run__byte_interp`); compile > 150 s or native run > 10 s
   (when bytecode finished) is `hang__*`.
3. **ref** (`ref__{value,exn}__<area>`): the trunk bytecode program must print
   the same as the same program compiled by the **OCaml 5.4.0 release**
   (`ocamlc` + `ocamlrun`, built by `build.sh` from the GitHub tarball into
   `<build>/ref54/inst`, independent of the patches; `ref54-patches/*.diff`
   backport trunk fixes for 5.4.0 bugs the oracle kept reporting, e.g. #14890
   min_int switch overflow). Catches front-end /
   Lambda bugs (matching.ml, translcore, tmc, value_rec_compiler, ...) that
   trunk bytecode and native share, which the diff oracle cannot see.
   Trunk-only stdlib functions the generator uses (`Int.popcount`,
   `leading_zeros`, `trailing_zeros`) are re-implemented from their trunk docs
   in `ocfuzz/src/compat54.ml` (compiled with `-open Compat54` for 5.4.0 only).
   Area: `all` = bytecode and every native config differ from 5.4.0 (shared
   front end), `byte` = only bytecode, else the tags (`b` + native tags). A
   5.4.0 compile error, crash or timeout skips the oracle. A ref finding can
   also be a 5.4.0 bug fixed in trunk or a documented behaviour change: check
   trunk's `Changes` before calling it real. Pre-check: 320 generated programs,
   0 differences.

Diff findings are reduced to the single first differing test function when
that still reproduces (`reduced` in stats.json). Each finding dir contains
`h.ml a.ml main.ml finding.json repro.sh`; `repro.sh <build>` recompiles with
the failing config and the bytecode reference.

## Known differences / masked bugs

No output normalisation besides the generator rules above. Two real compiler
bugs found while building the harness were masked in the generator so they
do not dominate the failure rate (both also in 5.4.0):

- `let f x = Some (let r = ref 0 in x)` -> closure ocamlopt `Fatal error:
  Selection.size_expr` (unused local ref directly inside an allocation). Masked:
  no ref-typed bindings in pure `let`s. `work/agents/harness/sizeexpr/`.
- `let f x = match x with _ -> 0 | (0 | 3) when true -> 1` -> ocamlc/ocamlopt
  `Fatal error: Matching: impossible empty matrix in a Total match`. Masked:
  only the final arm of a match may be irrefutable. `work/agents/harness/matching/`.

## Coverage

`build.sh ... coverage` runs `cov/build-cov.sh` for closure and flambda (in
parallel; flambda takes ~8 min). Host tools (OCaml 5.4.0 at `work/ref/boot`,
ppxlib 0.37 + bisect_ppx 2.8.3 with upstream PR #448 for ppxlib >= 0.36) are
built once into `cov/tools` (git-ignored, tarballs fetched on first use). A
ppxlib driver instruments `lambda/`, `middle_end/`, `asmcomp/` (+ `amd64/emit.mlp`
through cvt_emit) of the tree copy, recording the absolute paths of the files
in `<src>`; a vendored bisect runtime (`cov/rt`, writes only when `BISECT_FILE`
is set) is linked into compilerlibs. `ocfuzz` merges the settled
`$BISECT_FILE*.coverage` files every 20 cases and at exit
(`bisect-ppx-report merge`, inputs deleted). Gotcha (it failed the first gate): `bisect-ppx-report` `merge` and `cobertura`
rewrite source paths under the current directory to relative ones (cwd `work`
gives `trees/cov/...`), so the controller saw 0 target files. `coverage.json`
therefore points at `tools/bin/bisect-ppx-report-abs` (`cov/report.sh`), which
runs the real tool from `/` with path arguments made absolute, and ocfuzz runs
its merges from `/`. A 2-minute run reached e.g. cmmgen.ml 75%,
selectgen.ml 85%, matching.ml 66%, flambda inline_and_simplify.ml 83% of lines.

- trunk `-linscan` (not 5.4.0) miscompiles functions with several native atomic
  ops (`Atomic.compare_and_set` + `exchange` + `fetch_and_add` in one function:
  segfault or wrong atomic contents) on ~20% of generated cases. Not masked in
  the generator; instead `closure_linscan` runs on 1 case in 32
  (`LINSCAN_EVERY` in main.rs: raise it back to every case once fixed).
  10-line repro: `work/agents/steer-1/linscan-segv/min.ml`.

## Bugs seen so far

- `let f c = float_of_int (c asr 63)` gives -23 for c = -12 in native
  (closure and flambda, also 5.4.0), bytecode -1 (`asr` count 63 is allowed:
  stdlib.mli unspecified only for m > Sys.int_size). Not masked (rare).
- wrong code in match compilation (bytecode and native, also 5.4.0): a
  redundant row `| 6, true -> _` after `| _, true -> _` widens the integer switch
  on the first column and the upper bound check is dropped; `f 1000 false`
  jumps outside the jump table (segfault / ocamlrund `interp.c` assertion).
  `work/agents/harness/segv/min.ml`. Shows as `crash__run__byte_interp`. Not masked.
- the two masked crashes below.

## Stats keys

`cases`, `failures`, plus `gen_invalid` (ocamlc rejected the program: generator
bug, should be 0), `ref_timeout` (bytecode run timed out: skipped),
`ref_exn_cases` (cases where at least one test ended in an exception),
`reduced`, `cases_per_s`.

## Throughput

Measured on this box (16 cores): one run.sh alone, 3 workers: ~1.3 cases/s
(`OCFUZZ_JOBS=5`: ~1.7); four run.sh in parallel (the gate setup): ~0.48 cases/s
each, ~1.9 total, CPU-bound. Coverage build: ~0.65 cases/s per process with two
in parallel. A case is ~400 lines in 6-14 test functions and costs ~3 CPU-s
(7 compiles; flambda -O3 compiles dominate at 0.9-1.2 s each).

## Gate self-check (2026-10-21)

Fresh `build.sh work/src builds/gate` (~6 min), 4 x run.sh 240 s: 415 cases,
0 failures, 0 invalid. Coverage build (`builds/cov-dev`, ~8 min fresh), 2 x 240 s
with BISECT_FILE: 312 cases, 0 failures, coverage files merged, cobertura
report lists the target's lambda/middle_end/asmcomp files with hits.
