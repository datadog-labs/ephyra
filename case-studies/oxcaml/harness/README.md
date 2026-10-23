# oxfuzz: differential harness for oxcaml

Rust crate `oxfuzz/` (it uses ephyra's seeded `Source` for every random choice). It
generates typed OxCaml programs and compiles each one with several native flag sets
and with bytecode. Driven by `build.sh` and `run.sh` (the harness contract).

## Oracles

1. **diff (native vs bytecode)**: the reference is `ocamlc.opt` from the same build,
   run by `ocamlrun`. For each case the native compiler `ocamlopt.opt` (flambda2) runs
   twice with fixed flags and once with one rotating flag set:
   - always `-O3 -dflambda-invariants -runtime-variant d` (debug runtime, flambda invariant checks)
   - `-Oclassic` on half the cases (otherwise a second rotating set); its two known crashes would otherwise sit near the 1% gate limit
   - one of: default (no -O), `-O4` (reaper), `-O3` + the upstream oxfuzzer expert flags
     (match-in-match, n-way join, inline recursive, result types), `-O2 -regalloc gi`,
     `-O3 -regalloc irc`, `-O3 -regalloc ls`

   Stdout and the exit status must be identical. Signatures:
   - `diff__output__<area>`: stdout differs
   - `diff__crash__<area>`: native dies by signal or timeout and bytecode does not
   - `diff__exit__<area>`: only the exit status differs

   `<area>` is the focus feature of the first test whose output line differs:
   `int_arith float boxed_int unboxed variant_match record closure exn local loop_mutable string array`.
   It is `global_counter` for the final `cnt` line and `oxfuzzer` for programs from the upstream generator.
2. **toggle (switches that must not change results)**: a fourth native compile per
   case, `<base> <switch>...` with a base level (`-O3`, default, `-O2`, `-O3 -flambda2-reaper`,
   `-Oclassic`) and 1–3 switches from `TOGGLES` in `main.rs`. Most switches are off by
   default, so the other configs never reach that code: `-use-ssa` (with and without
   `-ssa-validate`/`-ssa-simplify`), `-x86-peephole-optimize`, `-cfg-merge-blocks`,
   `-cfg-block-layout`, `-cfg-copy-propagation`, `-reorder-blocks-random`,
   `-internal-assembler`, `-vectorize`, `-g` (DWARF, also with `-gdwarf-may-alter-codegen`),
   `-dcfg-invariants`, `-debug-long-frames-threshold 0`, `-dbranch-relaxation-max-displacement`,
   `-function-layout source`. The rest turn a default pass off (shrink-wrap, value
   propagation, CFG peephole, dead-trap elimination, join points, unboxing) or push a
   flambda2 knob to an extreme (inline depth 0 / huge, cse/join depth 0, cmm-safe-subst,
   phantom lets, ...). Output must equal bytecode. Switches in `RARE_TOGGLES`
   (`-flambda2-expert-inline-effects-in-cmm`, which crashes on most programs) are
   drawn 32x less often so they don't flood the findings.
   On a failure the case is localised: if the base level alone fails the same way it is
   recorded as an ordinary `diff__`/`crash__` finding; otherwise the first single switch
   that reproduces names it. Signatures: `toggle__<output|crash|exit>__<switch>` (e.g.
   `toggle__output__use_ssa`, `toggle__output__combo` when no single switch reproduces).
   A compiler crash that only one switch provokes is `crash__<kind>__<area>_with_<switch>`
   (the message-based area plus the switch), so it does not hide behind a known crash with
   the same message; it is recorded with the minimal failing flags. A crash that the base
   level alone also shows keeps the plain `crash__<kind>__<area>`.
   Not used: `-flambda2-kind-checks` (fails on ~75% of programs, "Unexpected approximation
   found when block approximation was expected"), `-verify-binary-emitter` (ARM64 only).
3. **crash (native compiler)**: ocamlc accepts the program but ocamlopt exits non-zero.
   Signatures are `crash__<kind>__<area>`. The kind is one of `fatal`, `assert`,
   `cmm_invariant`, `assembler`, `exception`, `signal`, `timeout` or `error`. The area is
   the first words of the message with digits and paths removed (for assertions, the
   source file stem).
   For an uncaught exception inside the compiler, the area is `<exn>_at_<module>_<function>`.
   It comes from the first non-stdlib backtrace frame; compiles run with `OCAMLRUNPARAM=b`.

`repro.sh <build>` recompiles the saved `prog.ml` with bytecode and the failing flag set.
- diff findings: exit 1 if the outputs still differ.
- crash findings: exit 1 if the compiler fails with the same message prefix (`grep -F`).
- exit 2 means it cannot decide (bytecode fails or times out).

## Generator (`oxfuzz/src/gen.rs`)

A program is a fixed prelude plus 3–8 top-level functions and 8–17 `test_<i>` functions.
Each test prints one hashed line: `t<i> <int>`. The prelude has:
- the types: variants with a mixed `float#` constructor, a mixed record with
  `float#`/`int64_u` fields, a flat float record, and an immediate-only variant
- observers `h_*`, which take `local_` parameters, plus `mix`

Generation is type-directed and valid by construction (the rules are in dossier §3).
- Expressions are pure and never raise. Division and shifts are guarded or masked.
  Float-to-int conversions are clamped (R6). Floats are observed through `hf`, which
  canonicalises NaN (R9). Because expressions are pure, the unspecified evaluation
  order (R1–R3) is never observable.
- Effects (`raise`, a global counter, mutation, effectful calls) only happen in
  sequenced statements.
- Modes:
  - `stack_` allocations, `exclave_` helpers and `local_` parameters are tracked by a
    "taint" flag, so local values never escape.
  - Applications that take a local argument get `[@nontail]`.
  - `let mutable` variables are never captured by closures.
- Constructs covered:
  - variants, nested and or-patterns, guards, match-in-match
  - records (mixed and flat-float), with updates and mutable fields
  - closures, partial application and composition
  - `try`, `match ... with exception`, `raise_notrace`
  - `for`/`while` loops with `let mutable` and refs
  - int/float arrays, strings, Int32/Int64, `float#`/`int64#`
  - int-valued uses of `Int8`/`Int16` (boxed and `int8#`/`int16#`), `Nativeint`,
    `Nativeint_u`/`Int32_u`, `Float32`/`Float32_u` (observed through `hf`, exact widening),
    `String.get_int{8,16,32,64}_*` and `Bytes.set_*` on fresh bytes at guarded indices,
    and unboxed products `#(..)` with `int#`/`float#` components
  - adjacent unchecked array stores inside a bounds guard (`Array.unsafe_set a (j+l)
    (Array.unsafe_get a (j+l') op ...)`), straight-line lanes for the `-vectorize` toggle
  - recursive functions with fuel, and `[@inline]`/`[@inline never]`/`[@specialise]`
- Each test starts with `Sys.opaque_identity` inputs, so it is not just constant folding.
- 15% of cases come from upstream's `oxfuzzer -seed N`. Those programs run under the
  configurations upstream does not test (-Oclassic, -O3, regalloc variants).

## Known differences and masks

- No output normalisation at all: outputs are compared byte for byte.
- **Known bug, partially avoided**: a `match` on a statically known `v2` constructor
  whose dead arms read a field of another kind (`float#` vs value) crashes `-Oclassic`
  with "Illegal move between registers of differing types", or with an assembler error.
  Minimal case: `agents/harness-002/cand-oclassic-illegal-move/min.ml`.
  - It hit about 7% of cases.
  - The generator now wraps literal `v2` scrutinees in `Sys.opaque_identity` 95% of the time.
  - `float#`/`int64#`-typed match expressions are generated at 1/20 of their normal rate.
- Not enabled, because the noise is too high (dossier §4):
  - `-dcmm-invariants`: a continuation-arity failure on about 10% of oxfuzzer programs and on the compiler's own sources
  - `-dflambda-heavy-invariants`: "Cannot rebind" on about 40% of programs
- Rejected cases are counted, not failed (extra keys in `stats.json`):
  - `gen_reject`: bytecode does not compile. These are generator bugs, sampled into `<out>/rejects/`.
  - `bytecode_timeout`: the program runs longer than 10 s
  - `bytecode_nonzero_exit`: informational

## Throughput

About 0.37 cases/s per process with 4 processes in parallel on 16 CPUs. One case is
1 bytecode compile and run plus 4 native compiles and runs (3 configs + 1 toggle), on programs of about
150–300 lines.

## Coverage

`build.sh <src> <build> coverage` does the normal build, then a second tree
`<build>/covsrc` configured with `--enable-coverage`. Only the boot compiler is built
there: `_build/default/boot_ocamlopt.exe`, which is the same compiler sources built by
OCaml 5.4 and instrumented with bisect_ppx. The bisect toolchain is in
`work/tools/cov` (`work/tools/build-cov.sh`): ppxlib 0.39 (0.37 cannot migrate labelled tuples) and bisect_ppx PR #448,
which supports ppxlib ≥ 0.36.

When `covsrc` exists, `run.sh` uses that compiler for the native compiles and points
`OCAMLLIB` at the installed stdlib. The runner merges the `.coverage` files next to
`$BISECT_FILE` every 25 cases once there are 50 or more.

## Tools

`tools/reduce.py prog.ml test.sh out.ml` is a greedy item- and line-level reducer.
`oxfuzz gen <seed>` prints a program.
`oxfuzz repro-script <sig> <flags> diff|crash [key]` prints a repro.sh.

Coverage-tree tweaks (in `build.sh`, applied to `covsrc` only):
- `dune-workspace`: the boot context's `OCAMLPARAM` is set to `_,warn-error=-a`, because
  the instrumentation triggers warnings 51 and 71.
- root `dune`: the manual `unix.cmx?a` link flag is dropped, because the bisect runtime
  already links unix.

Bisect file names are relative to `covsrc` (e.g. `middle_end/flambda2/...`). Some are
bare basenames, because dune uses `copy_files`.

Measured: 2×240 s on the coverage build gave about 0.17 cases/s per process and 41.6%
bisect point coverage (46k of 111k points).

## Findings so far (both are -Oclassic only, and both reproduce on base)

1. `crash__fatal__illegal_move_between_registers`: the minimal case is in
   `agents/harness-002/cand-oclassic-illegal-move/min.ml`. It is a 3-line program and
   is partially avoided by the generator (see above).
2. `crash__exception__invalid_argument_at_cfg_selectgen_emit_expr_raise`: `Array.iter2`
   fails in `Cfg_selectgen.emit_expr_raise`. It is about 0.6% of cases and is not
   masked. A partly reduced case is in `agents/harness-002/cand-oclassic-iter2/`.

Gate check on base (4×240 s, last run): 327 cases, 2 failures (0.61%), both finding 2.

### Absolute source paths (coverage gate fix)

By default bisect records dune-relative names (`middle_end/...`), and files copied by
`copy_files#` appear under bare basenames. `bisect-ppx-report cobertura`, when run from
another directory, then cannot find the sources. Two changes fix this:
- `tools/bisect-abs-path.py` patches the bisect_ppx instrumenter, and
  `work/tools/build-cov.sh` applies the patch. When `BISECT_SOURCE_ROOT` is set, every
  file is recorded as `<root>/<path>`. The path follows the copied file's
  `# 1 "dir/f.ml"` line directive.
- `build.sh` sets `BISECT_SOURCE_ROOT=<build>/covsrc` in the boot context's
  `dune-workspace` env-vars.

`bisect-ppx-report merge` rewrites absolute paths under its cwd to relative ones, so
oxfuzz runs its own merges with cwd `/`. I checked this end to end: a 2×120 s run on the
coverage build, followed by a merge and cobertura from `/agent-run`,
`/agent-run/work` and `/`, mapped 451 of 451 files to `work/src`.
