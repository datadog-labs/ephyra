# tyz: generated typed Python against ty

tyz tests [ty](https://github.com/astral-sh/ty) (Astral's type checker, in the ruff repo) with Python
programs that are typed by construction and run under CPython. Each program's runtime behaviour is the
reference ty is checked against. ty's own fuzzer (`ruff/python/py-fuzzer`) only checks for crashes on
random syntax, and its quickcheck property tests (`ty_python_semantic/src/types/property_tests.rs`) cover
the type algebra. tyz aims at inference and narrowing on whole programs.

## Programs (`src/pygen.rs`)

A program has enums (`Enum`, `IntEnum`, `StrEnum`), protocols (some `@runtime_checkable`), TypedDicts,
classes (plain, with inheritance and covariant overrides; `@dataclass` with `frozen`/`slots`/`kw_only`;
`NamedTuple`; PEP 695 generic classes), functions (some PEP 695 generic, bounded, `TypeIs`/`TypeGuard`
guards), and `main()`, which calls every function one to three times with generated arguments. Union
parameters therefore see several runtime types. Function bodies use narrowing (`is None`, `isinstance`,
`type(x) is`, enum identity, truthiness, walrus, tuple elements, TypedDict keys, user guards), `match`
(class, value, sequence, mapping and capture patterns), loops whose accumulator the body widens and
narrows (ty's fixpoint iteration), `try`/`except`/`else`/`finally`, `with`, closures, comprehensions and
unpacking.

The model (`Ty`, `Model::sub`) is coarser than the typing spec. A judgement in the model must hold in
ty, never the other way round. Each place where ty's semantics forced the model to be more careful has a
comment: `float` is `int | float`; non-final classes may have `Sequence`/`Mapping` subclasses; value
patterns use `__eq__`; unannotated bindings take the inferred type; and so on.

The output is a tree of lines (`src/tree.rs`). The shrinker drops runs of siblings, drops nodes,
hoists blocks and empties blocks, without knowing Python. A candidate that fails differently is
rejected.

## Oracles (`src/main.rs`)

`eval_batch` runs a batch of programs under CPython 3.13 (`py/runner.py`, each in a forked child, with
a 3 s limit). `_probe(k, e)` (`py/_rt.py`) records the runtime type of `e` as a type expression:
literals for ints, strs, bytes and enum members; `TypeOf[0.5]` for floats; tuples element-wise; class
instances as `Intersection[C, Not[X], ...]` over every program and builtin class `X` the value is not an
instance of, plus `Not[AlwaysFalsy]` or `Not[AlwaysTruthy]`; and `?` (no assertion) for dicts, which
could be TypedDicts. Programs that raise are `Invalid` (generator misses, counted). For the others, a
check file gets, after each observed probe (union of the observed types `U`):

```python
static_assert(not is_disjoint_from(U, TypeOf[e]))   # sound__disjoint: ty rules out the value itself
static_assert(is_assignable_to(U, TypeOf[e]))       # sound__assign: weaker; fails when ty cannot prove a coarse U fits
```

ty checks all the check files of a batch in one `ty check --project .` run. Verdicts, first match wins:
`crash__panic__<file>_<line>` (a `fatal[panic]` diagnostic), `crash__exit__*` / `crash__hang__ty` (the
whole batch), `fp__<rule>` (an error on a program line: the program ran, and the model says it is
well-typed), `harness__assert__<rule>` (an error on an inserted line other than a failed assertion),
`sound__disjoint`, `sound__assign`.

`known.json` lists triaged findings: a signature prefix plus a substring of the message or of an error.
They are counted as `known__<name>` and not saved again.

## Commands

```sh
. ./env.sh                       # TYZ_TY (ty binary), TYZ_PY (python3.13)
cargo build --release
./target/release/tyz search <seed> <secs> <out>   # TYZ_WORKERS (4), TYZ_BATCH (48), TYZ_MAX_CASES
./target/release/tyz minimize <out>/findings/<sig>/<n> [secs]   # min.py, min_check.py, finding.json
./target/release/tyz eval prog.py                 # one program: check file and verdict
./target/release/tyz gen <seed> <n> <dir>         # samples
./show.sh <finding-dir>...                        # minimized program and messages
./cov.sh <seed> <secs> <out>                      # search against a coverage build; <out>/cov-files.txt
```

`search` saves at most three originals per signature (`findings/<sig>/<n>/{original.py,tree.json,
finding.json}`) and rewrites `stats.json` after every batch: cases, invalid (by exception), passes,
probes, asserts, and failures by signature. Minimizing runs separately, so the search doesn't stall.

ty builds used locally (`ruff/` checkout at the monorepo root, gitignored):
`target/release/ty` (release), `target-da/release/ty` (`CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true`,
overflow checks), `target-cov/aarch64-apple-darwin/release/ty` (`-C instrument-coverage`, no LTO;
`cov.sh`).

## ephyra-agent contract

`build.sh <src> <build> [coverage]` builds ty from a ruff checkout `<src>` into `<build>/bin/ty` (release,
no LTO, debug assertions and overflow checks; with `coverage`, `-C instrument-coverage` and
`<build>/coverage.json` instead), tyz into `<build>/bin/tyz` (against ephyra at `$EPHYRA`, default
`/opt/ephyra`), and links a CPython 3.13+ as `<build>/bin/py` (`$TYZ_PY`, or a uv install under
`/agent-run/work/.python`). `run.sh <build> <seed> <secs> <out>` runs `tyz search` with
`TYZ_WORKERS` (6) workers. `stats.json` has `failures` (non-known failures). Each finding has
`finding.json` (`signature`, `oracle`, `message`, `seed`), `original.py`, `tree.json` and `repro.sh`
(`repro.sh <build>` runs `tyz repro`: exit 1 when `original.py` still fails with the signature,
0 when it passes, 2 when it no longer runs under CPython). `known.json` is compiled in (`TYZ_KNOWN`
overrides it).

## Results

See `../bugs/STATUS.md`. Generator rounds are in `../rounds/<n>/` (ROUND-LOG.md), scored by covered
lines of `ty_python_semantic` over a fixed number of programs (`TYZ_MAX_CASES=4800 ./cov.sh 11 ...`).
