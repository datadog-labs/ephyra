# ocfuzz: ephyra-based differential + crash harness for the OCaml native compiler

Target: OCaml trunk (`work/src` + `patches/`), native backends (closure and
flambda1) against the bytecode compiler of the same tree. Run 3 port of run 2's
ocfuzz (`work/reference/harness`) onto **ephyra** (`/opt/ephyra`).

## Layout

- `build.sh <src> <build> [coverage]` builds two compilers from `<src>` in parallel:
  `<build>/closure/inst` (default configure) and `<build>/flambda/inst`
  (`--enable-flambda --enable-flambda-invariants`), then the Rust driver
  `<build>/bin/ocfuzz` (`ocfuzz/`, depends on ephyra by path), and (`ref54.sh`,
  both modes, once) the OCaml 5.4.0 release bytecode compiler into
  `<build>/ref54/inst` (tarball from GitHub cached in `<build>/.cache`,
  `ref54-patches/` applied). Incremental: the
  tree is rsynced into `<build>/<cfg>/src` and `make` reruns. With `coverage`,
  `cov/build-cov.sh` builds the same two compilers with `lambda/`,
  `middle_end/`, `asmcomp/` instrumented by bisect_ppx and writes
  `<build>/coverage.json`.
- `run.sh <build> <seed> <seconds> <out>` = `ocfuzz search`: 3 worker threads
  (`OCFUZZ_JOBS`), each running a sequence of `ephyra::check` calls of 40 tests
  (bounded by a 120 s budget and the run deadline). Each test is one case;
  a case compiles its 7 configs in parallel.
- `ocfuzz gen --seed S --dir D` writes the program drawn from
  `ephyra::Source::new(S, 10)`; `ocfuzz check --build B --dir D` evaluates a
  case directory with all configs (handy for triage).
- `tools/linereduce.py <file> <cmd>`: greedy line-deletion reducer.

## ephyra integration

- `gen.rs` draws every decision from the test's `ephyra::Source` (`Rng` wraps
  `&mut Source`), so a program is a pure function of ephyra's seed/iteration.
- `CaseGen` implements `ephyra::Gen`: `generate` = one program (plus a drawn
  bit choosing `-linscan` for 1 case in 4), `shrink` = deletions only: keep a
  single test, drop half the tests, drop one test, drop one top-level item of
  A, drop one line of a test. A candidate ocamlc rejects does not reproduce.
- Property: fresh case -> all configs; any finding panics at a per-check-kind
  site (`fail()`), so ephyra's "same failure" = same check kind. Once a case
  failed, the property switches to targeted mode for the rest of the check
  (ephyra's rerun and every shrink candidate): only bytecode + the failing
  config are compiled (bytecode first; native only if it type-checks), and a
  candidate counts as failing only for the same check kind, same config and
  (except diff) the same signature. The smallest failing case is written as
  the finding; other findings of the same case are written unshrunk.
- Shrinking budget: 400 steps, at most 120 s and never past the run deadline.
- `EPHYRA_REPORT_JSON` is honoured by ephyra itself (not set by the harness);
  stats.json has `checks` and `ephyra_tests` (sum of `report.tests`).

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
bounded local recursion, Atomic get/set/exchange/compare_and_set/fetch_and_add/
incr/decr/update, the same through `Atomic.Loc` on `[@atomic]` record fields
(`H.ar`, `[%atomic.loc r.H.ai]`, plus plain reads/writes of those fields) and
on `Atomic.Array` (incl. `unsafe_*`, indices always in bounds) (run 3),
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
GADTs (run 3, steering round 2; aimed at the 2026-09 matching/translcore
changes "immediate field reads", "ignore GADT equations when classifying
record fields", "do not trust partial GADT specialization"): local functions
matching a witness `H.w` (`WI/WS/WF/WL/WP`: int, string, float, int list,
int*float) with a payload of the witnessed type, shaped as in
`testsuite/tests/basic/gadt_immediate_fields.ml` / `typeopt/partial_gadt.ml`:
tuple, `match w, x`, constructor or record of a local module type mentioning
the abstract type, `match w` with the payload parameter rebound in the arm,
`type a.` annotation, and a GADT pattern on a parameter (partial). Arm subsets
in random order, optional wildcard (else Match_failure, call alone in its let),
calls direct or through the existential `H.gb`. Arms print the payload, half
of them after `Gc.minor ()`; strings are sometimes fresh heap strings
(`H.fresh_str`). `Gc.minor ();` is also a rare plain statement.

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
   -function-sections`), replaced by `cl` closure_linscan (the same plus
   `-linscan`) on 1 case in 4, `o3` flambda `-O3`, `oc` flambda `-Oclassic -g`,
   `ox` flambda `-O3 -unbox-closures -rounds 4 -inline-max-depth 4
   -remove-unused-arguments -runtime-variant d`. One diff finding per case.
2. **crash** (`crash__compile__<build>_<area>`, `crash__run__<build>_<area>`,
   `hang__{compile,run}__<build>`): the bytecode compiler accepted the program,
   so every native compile must succeed (area = the `>> Fatal error:` word or
   the compiler source file of an `Assert_failure`); ocamlc itself crashing
   gives `crash__compile__byte_<area>`; a binary killed by a signal or printing
   a runtime assertion is `crash__run__<build>_<area>` (area = runtime source
   file of the assertion, `fatal_error`, or `sigN`); compile > 150 s or
   native run > 10 s (when bytecode finished) is `hang__*`.
3. **ref** (`ref__{value,exn}__<area>`, run 3 steering round 2, ported from run
   2): the trunk bytecode program must print the same as the same program
   compiled by the **OCaml 5.4.0 release** (`ocamlc` + `ocamlrun`, built by
   `ref54.sh`, independent of `patches/`; `ref54-patches/*.diff` backport
   trunk fixes of matching bugs 5.4.0 shares: #15185-like
   `as_interval_canfail`, #14890 min_int switch). Catches front-end / Lambda
   bugs (matching.ml, translcore, tmc, value_rec_compiler, simplif, ...) that
   trunk bytecode and native share, which the diff oracle cannot see. The
   language used is the same in both; trunk-only stdlib functions
   (`Int.popcount/leading_zeros/trailing_zeros`, `Atomic.Array`,
   `Atomic.update`) are re-implemented from their trunk docs in
   `ocfuzz/src/compat54.ml` (compiled with `-open Compat54` for 5.4.0 only).
   Area: `all` = bytecode and every native config differ from 5.4.0 (shared
   front end), `byte` = only bytecode, else the tags (`b` + native tags). A
   5.4.0 compile error, crash or timeout skips the oracle (stat `ref54_skip`).
   A ref finding can also be a 5.4.0 bug fixed in trunk or a documented
   behaviour change: check trunk's `Changes` before calling it real.

4. **meta** (`meta__{value,exn}__<wrap>_<tags>`, run 3 steering round 3):
   a metamorphic relation on the module structure. Each case draws a `wrap`
   (`Case::wrap`, written to the case dir as `wrap`); three extra configs
   (`meta_byte` = ocamlc + ocamlrund, `meta_closure`, `meta_o3` = flambda
   `-O3`) compile the same program with A's text re-wrapped (`meta_a` in
   main.rs) and must print exactly what the *original* program prints in
   bytecode. Wrapping a structure does not change what its items compute
   (the manual's module language: `include` of a functor application,
   recursive modules, first-class module packing all evaluate the same
   structure items in order). Wraps: `fun` = `module Body () = struct <A>
   end include Body ()` (A's top level becomes a functor body: translmod's
   functor path, flambda/closure functor inlining, no static allocation of
   A's globals); `rec` = `module Body0 = struct <A> end include Body0`, then
   `module rec Body : sig <A's function-typed vals from ocamlc -i> end =
   struct include Body0 end` (a safe recursive module: `CamlinternalMod`
   `init_mod`/`update_mod`, translmod `compile_recmodule`) and every function
   rebound as `let f = Body.f`, so Main calls the patched closures (lazies
   are left out: update_mod copies an unforced lazy, forcing both copies
   would run it twice); `fcm` = `module type SIG = sig <ocamlc -i> end
   include (val (module struct <A> end : SIG))`; `main` = A unchanged, Main's
   whole body (test functions and the `let () = H.run ...` lines) in
   `module Body () = struct <Main> end module _ = Body ()`, so the tests
   become local closures of a functor body. The original text is preceded by
   `# 1 "a.ml"` / `# 1 "main.ml"` so Match_failure/Assert_failure locations
   do not move (without it every Match_failure was a false positive).
   Tags: `b`/`c`/`o3` = the meta configs that differ. Bugs shared by
   bytecode and native in lambda/ (translmod, translcore, simplif) show as
   `..._b_c_o3`; a native-only bug that needs the wrapped shape shows as
   `..._c` / `..._o3`. A wrapped program that does not type-check (or
   `ocamlc -i` failing) skips the oracle (stat `meta_skip`, should be ~0); a
   crash or hang in a meta config is reported by the crash oracle with the
   usual signature. Finding dirs of meta configs also hold `a_meta.ml` and
   `main_meta.ml` (the files the meta config compiled); repro.sh compiles
   them as `a.ml`/`main.ml` (MetaDiff: next to the original bytecode run).
   Validated with a planted difference (shrunk to ~25-line Main, repro.sh
   exits 1).

The native configs `closure_inline` and `flambda_o3x` run with
`OCAMLRUNPARAM=s=32k` (minor heap 32k words, default 256k: more minor
collections inside the short tests, so missing GC roots show; repro.sh sets
the same).

OxCaml (steering round 1) failed the gate: its own -O3 CAS miscompile made
> 1% of cases fail (`work/agents/steer-1/oxcaml-cas`).

Each finding dir contains `h.ml a.ml main.ml compat54.ml finding.json repro.sh`;
`repro.sh <build>` recompiles with the failing config and the bytecode
reference. `seed` in finding.json is the ephyra check seed (`ephyra:<n>`).

## Known differences / masked bugs

No output normalisation besides the generator rules above (the debug runtime
banner is silenced with `OCAMLRUNPARAM=v=0`; only stdout and exit status are
compared). Generator-level masks carried over from run 2 (both still open):

- `let f x = Some (let r = ref 0 in x)` -> closure `Fatal error:
  Selection.size_expr` (#15154, patched in `patches/` now, mask kept).
- `let f x = match x with _ -> 0 | (0 | 3) when true -> 1` -> `Fatal error:
  Matching: impossible empty matrix in a Total match` (#15171). Masked: only
  the final arm of a match may be irrefutable.

## Coverage

`build.sh ... coverage` runs `cov/build-cov.sh` for closure and flambda (in
parallel; flambda takes ~8 min). Host tools (OCaml 5.4.0 at `work/ref/boot`,
ppxlib 0.37 + bisect_ppx 2.8.3 with upstream PR #448 for ppxlib >= 0.36) are
built once into `cov/tools` (git-ignored, tarballs fetched on first use). A
ppxlib driver instruments `lambda/`, `middle_end/`, `asmcomp/` (+ `amd64/emit.mlp`
through cvt_emit) of the tree copy, recording the absolute paths of the files
in `<src>`; a vendored bisect runtime (`cov/rt`, writes only when `BISECT_FILE`
is set) is linked into compilerlibs. `ocfuzz` merges the settled
`$BISECT_FILE*.coverage` files after every ephyra check (~40 cases per worker) and at exit
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

## Stats keys

`cases`, `failures`, `gen_invalid` (ocamlc rejected a fresh program: generator
bug, should be 0), `ref_timeout` (bytecode run timed out: skipped),
`ref54_skip` (OCaml 5.4.0 rejected / crashed on / timed out on the program:
ref oracle skipped; should be ~0),
`meta_skip` (meta oracle skipped, see above), `ref_exn_cases`, `checks` (ephyra::check calls), `ephyra_tests`,
`shrink_evals` (targeted evaluations while shrinking), `cases_per_s`.

## Throughput

Measured 2026-10-27 on this box (16 cores): one run.sh alone, `OCFUZZ_JOBS=5`:
~1.5 cases/s (291 cases in 191 s, 0 failures). Four run.sh in parallel (the
gate setup, 3 workers each): ~0.39 cases/s each, ~1.55 total (363 cases,
0 failures; ephyra `tests` = cases). Coverage build, two in parallel: ~0.55
cases/s each; 262 cases gave 67% of the instrumented lines (lambda/,
middle_end/, asmcomp/: 16206 of 24064). A case is ~400 lines in 6-14 test
functions; 7 compiles, flambda -O3 compiles dominate.
