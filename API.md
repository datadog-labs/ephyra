# ephyra API reference

A compact inventory of the public API, for people and agents writing
tests. The [library README](crates/ephyra/README.md) explains the ideas; rustdoc
(`cargo doc -p ephyra --open`) has every item. Names not listed here
usually don't exist: see [Things that do not exist](#things-that-do-not-exist).

```rust
use ephyra::prelude::*;           // Gen, GenExt, Source, Config, check, target, assume!, ...
use ephyra::{gens, check_shrinker, Run};
```

## Generators: `Source`, `gens`, `GenExt`

A generator is any `Fn(&mut Source) -> T + Send + Sync`, or a value from
`gens::*`. Every generator owns a shrinker and a mutator.

`Source` (seeded; use no other randomness in a generator):

```rust
g.int(0u32..=9)  g.int(..)  g.index(len)  g.bool()  g.bool_with(0.2)
g.pick(&[a, b])  g.weighted(&[(3, a), (1, b)])  g.f64()  g.f64_bits()
g.size()         g.draw(&other_gen)            g.discard("reason")
```

Constructors in `gens`:

| Constructor | Notes |
|---|---|
| `int(range)` | `i8`..`i64`, `isize`, `u8`..`u64`, `usize`. No `i128`/`u128`: draw an `i64` and widen. |
| `edges(range)` | Only `lo, hi, lo+1, hi-1, 0, 1, -1`; mix with `int` through `frequency`. |
| `bool()`, `bool_with(p)` | |
| `f64()`, `f32()`, `f64_bits()`, `f32_bits()`, `f64_range(a..=b)` | `f64()` is uniform in `[0, 1)` only. `f64_bits()` draws a uniform bit pattern: NaN about 1 in 2,000, infinities and zeros practically never; add them with `frequency` and `constant(f64::INFINITY)`. |
| `char()`, `ascii_char()`, `alphanumeric()` | `char()` is any scalar value, biased to ASCII. |
| `string()`, `string_len(chars, range)` | |
| `vec(elem)`, then `.len(range)` | One element generator. |
| `option(g)`, `result(ok, err)` | |
| `constant(v)`, `pick_from(vec![..])` | |
| `one_of(vec![g.boxed(), ..])`, `frequency(vec![(w, g.boxed()), ..])` | Call `.boxed()` on the result for a `BoxGen`. |
| `recursive(depth, leaf, \|inner\| ..)` | A bounded recursive type. |
| `sized(\|size\| gen)` | Has no shrinker. |
| `dependent(outer, \|a\| inner)` | Returns `(A, B)`. |
| tuples `(g1, g2, g3)`, arrays | Generators too. |
| `ephyra::any::<T>()` | The `Arbitrary` default for std types and derived types. |

**Size.** At iteration `i` the size is `floor(log2(i + 1))`, capped at
`max_size` (100). With the default 10,000 tests it never exceeds 13, so
`vec(x)` and `string()` without an explicit length stay short. Write
`.len(0..=N)` or `string_len(c, 0..=N)` when long inputs matter.

Combinators (`GenExt`):

| Combinator | Keeps shrink and mutate? | For |
|---|---|---|
| `.map(f)` | No: shrinks nothing, mutation regenerates | Cheap reshaping, then `.with_shrink` |
| `.and_then(f)` | No | Rarely; prefer `dependent` |
| `.iso(forward, back)` | Yes, through a bijection | Newtypes and wrappers |
| `.dependent(\|a\| gen_b)` | Yes; returns `(A, B)` | A query over a schema, an index into a length |
| `.filter("why", pred)` | Filters candidates and mutants too; discards after 100 misses | Rare exclusions; construct valid values instead |
| `.with_shrink(f)` | Adds candidates, tried first | A shrinker on the final type |
| `.with_mutate(f)` | Replaces the mutator | A custom mutator |
| `.collect::<C>()` | Yes | `Vec<(K, V)>` into a map |
| `.boxed()` | Yes | Storing generators, type erasure |

A shrinker on the final type, the usual fix when a report says
`shrinking: 0 of 0 candidates`:

```rust
use std::ops::ControlFlow;
fn shrink_expr(e: &Expr, visit: &mut dyn FnMut(Expr) -> ControlFlow<()>) -> ControlFlow<()> {
    if let Expr::Bin(op, a, b) = e {
        visit((**a).clone())?;               // a child replaces the node
        visit((**b).clone())?;
        shrink_expr(a, &mut |a2| visit(Expr::Bin(*op, Box::new(a2), b.clone())))?;
        shrink_expr(b, &mut |b2| visit(Expr::Bin(*op, a.clone(), Box::new(b2))))?;
    }
    ControlFlow::Continue(())
}
let exprs = expr_gen.with_shrink(shrink_expr);
```

A shrinker makes only strictly smaller candidates, never its input, and
propagates `?`. `#[ephyra::shrinker]` turns an eager `fn(&T) -> Vec<T>` into
one.

Derive:

```rust
#[derive(Clone, Debug, ephyra::Arbitrary)]
enum Op {
    Pop,                                              // the simplest variant first
    Push(#[ephyra(gen = gens::int(0i32..=99))] i32),
    Get(#[ephyra(gen = gens::int(0usize..=9))] usize),
}
// gens::vec(ephyra::any::<Op>()).len(0..=40)
```

Recursive derived types have no depth bound; use `gens::recursive`.

Verifying generators: `check_generator`, `check_shrinker` and `check_mutator`
run through `check`, so they shrink and replay like a property. Mark a
generator and its support predicate with `#[ephyra::generator(pred, metrics
= f)]` and `ephyra check` (the CLI) verifies all of them.

## Properties

A property takes the generated value and returns `()`, `bool`,
`Result<_, E: Debug>` or `TestResult`. A panic anywhere in it, including in
the code under test, is a failure, with its message and location.

- Prefer `()` with `assert!` and `assert_eq!` (which prints both values).
- Skip inputs that don't apply with `ephyra::assume!(cond)`; too many
  discards end the run with `GaveUp`.
- Shrinking keeps "the same failure": the same kind and, for a panic, the
  same file and line. Give each check its own `assert!` line so shrinking
  never drifts from one bug to another. An `Err` or `TestResult::fail` from
  two different checks counts as the same failure.
- `ephyra::target(score)` inside a property steers the targeted search
  toward higher scores; `ephyra::cover(cond, label)` checks that a share of
  inputs has a property.
- Not caught: stack overflow, `abort`, segmentation faults, infinite loops,
  out-of-memory. Run code that can do these in a child process with a
  timeout. `Config::timeout_ms` limits the whole run, not one test.

## Running

```rust
let config = Config::default()
    .tests(20_000)                           // passing tests to require
    .seed(7)                                 // a reproducible run
    .budget(Duration::from_secs(600))        // stop the random phase early
    .name("decode_never_panics")             // the regressions file's name
    .no_regressions();                       // don't save or replay .ephyra/regressions
let report = check(&config, generator, property);
report.assert_passed();
```

The attribute form gives one `#[test]` per property:

```rust
#[ephyra::test(gen = pairs(), tests = 5000)]        // also: seed = N, max_size = N,
fn decode_inverts_encode(p: Vec<(String, String)>) { // config = expr, search = expr
    assert_eq!(decode(&encode(&p)), Ok(p));
}
```

Environment variables (no recompilation):

| Variable | Effect |
|---|---|
| `EPHYRA_TESTS`, `EPHYRA_SEED`, `EPHYRA_BUDGET_MS`, `EPHYRA_MAX_SIZE` | The budget and the seed |
| `EPHYRA_REPLAY=seed:iteration` | Test that one input first; the report prints the line |
| `EPHYRA_FAILURES=N` | Collect up to N distinct failures instead of stopping at the first |
| `EPHYRA_SHRINK=0` | Report the original failing input |
| `EPHYRA_REGRESSIONS=0` | Don't save or replay remembered failures |
| `EPHYRA_REPORT_JSON=path` | Append each report as a JSON line |
| `EPHYRA_COVERAGE_HTML=dir` | Write the coverage grid page (instrumented builds) |

A failing input is saved to `.ephyra/regressions/<name>.jsonl` under the
working directory and replayed first next time; a failure at "iteration 0"
on a rerun may be that replay. Delete the file, or set
`EPHYRA_REGRESSIONS=0`, after fixing the bug.

## Many failures in one run

By default a run stops at the first failure. To collect several distinct
bugs from one long search:

```rust
let config = Config::default().failures(10);          // or EPHYRA_FAILURES=10
#[ephyra::test(gen = cases(), failures = 10)]         // the attribute form
```

The run goes on until it has `n` distinct failures (or the budget ends).
Each new failure is shrunk while keeping its signature, remembered as a
regression, and reported with its own `reproduce:` line; repeats are only
counted. `report.failures` lists them all (`signature`, `count`, `failure`);
`report.failure` is still the first.

A failure's signature, first match wins:

1. `ephyra::signature("display not reparsable: SHOW")` called inside the
   property for that case;
2. `Config::signature(|f: &FailureInfo| ...)` (`f.kind`, `f.message`,
   `f.location`, `f.value`);
3. the default: the panic location `file:line:col`, `returned false`, or the
   `Err` message with numbers and quoted values blanked.

Give each kind of check its own `assert!` line, or name it with
`signature()`, so different bugs don't merge. Manual `Run` loops keep going
past failures too (`while run.wants_more()`); only the first failure is
localized when coverage is on.

## Reading a report

```text
ephyra: FAILED, property panicked: assertion `left == right` failed: step 4: Get(1)
  left: Some(0)
 right: Some(1)
  at tests/model.rs:36:31
  counterexample: (2, [Push(0), Push(0), Pop, Push(1), Get(1)])
  original:       (2, [Get(8), Push(59), ... 32 ops]) (size 0, iteration 0)
  shrinking: 13 of 179 candidates accepted in 0.6 ms
  reproduce: EPHYRA_REPLAY=5713660488102024575:0
```

In code: `report.status` (`Passed`, `Failed`, `GaveUp`, `TimedOut`,
`Aborted`), `report.passed()`, `report.counterexample()`, `report.failure`
(`kind`, `message`, `location`, `original`, `shrunk`, `replay`, `flaky`),
`report.tests`, `report.iterations`, `report.discarded`, `report.labels`,
`report.health`, `report.to_json()`.

Health warnings worth acting on (`report.health[i].kind`):

| Kind | Meaning |
|---|---|
| `flaky` | The failure didn't reproduce on a rerun, so it wasn't shrunk: look for state, time or threads |
| `shrink_budget` | Shrinking stopped early; raise `config.shrink.max_steps` |
| `cover` | A `cover(cond, label)` share was below 5% |
| `coverage_plateau` | Coverage stopped growing: the generator is too narrow |
| `slow_generation` | Generation dominates the time |
| `config_env` | An `EPHYRA_*` variable has a bad value |

## Your own loop: `Run`

`check` is this loop:

```rust
let mut run = Run::new(&config, generator, property);
if !run.remembered() {                      // replays EPHYRA_REPLAY and saved regressions
    while run.wants_more() {                // tests, budget, discards, a failure
        let Some(input) = run.draw() else { continue };
        let tested = run.test(&input);      // .passed() .failed() .novelty() .score()
        if tested.passed() { run.keep(input.value, &tested); }
    }
}
let report = run.finish();                  // shrinks, localizes, saves, reports
```

Other steps: `run.mutate(&parent)`, `run.pool_pick()`,
`run.keep_always(value, score)`, `run.failure()`, `run.shrink()`,
`run.reproduces(&candidate)`, `run.shrunk_to(minimal)`, `run.report()` and,
with coverage, `run.suspects()`. [examples/custom_loop.rs](crates/ephyra/examples/custom_loop.rs)
is a complete one.

A loop that alternates fresh draws with mutants of inputs that reached new
code, through the run's own pool:

```rust
let mut mutate = false;
while run.wants_more() {
    mutate = !mutate;
    let parent = if mutate { run.pool_pick() } else { None };
    let input = match &parent { Some(p) => run.mutate(p), None => run.draw() };
    let Some(input) = input else { continue };
    let tested = run.test(&input);
    if tested.failed() { break; }
    if tested.novelty() > 0 { run.keep_always(input.value, f64::from(tested.novelty())); }
}
```

Shipped loops (`ephyra::loops`, [examples/shipped_loops.rs](crates/ephyra/examples/shipped_loops.rs))
take `(&mut run, &mut pool, fuel)`: `fuzz` (inputs reaching new coverage
become seeds; plain random testing without coverage), `target` (seeds
ranked by `target(score)`), `perf_fuzz` (toward slow inputs), and with the
`coverage` feature `localize(&mut run, fuel, patience)`. `fuel` counts draws
and mutations and ignores `Config::tests`; call `run.finish()` afterwards.

Pools: `Fifo::new()`, `Fifo::with_energy(n)`, `Heap::new()`, `Filo`,
`Singleton::new(policy)`, `DoubleQueue::new()`. Each seed gets 100 mutations
by default, after which `fuzz` degrades to fresh draws; seed the pool with
the project's own inputs and raise the energy:

```rust
use ephyra::loops::{self, SeedPool, Validity};
let mut pool = loops::Fifo::with_energy(20_000);
pool.invest(fixture, Validity { valid: true, score: 1.0 });
loops::fuzz(&mut run, &mut pool, 200_000);
```

## Coverage (in-process, no LLVM tools)

1. `features = ["coverage"]` on the ephyra dependency.
2. `-C instrument-coverage` for the whole build, for example in the test
   crate's `.cargo/config.toml`:

   ```toml
   [build]
   rustflags = ["-C", "instrument-coverage"]
   [env]
   LLVM_PROFILE_FILE = { value = "target/profraw/%m_%p.profraw", relative = true }
   ```

   An exported `RUSTFLAGS` overrides `[build] rustflags` and coverage turns
   off silently: check `echo "$RUSTFLAGS"`, and if it is set, add the flag
   there. Point `LLVM_PROFILE_FILE` away from the code under test, or build
   scripts leave `default_*.profraw` files in its source directories.
3. Say which crate to track when the code under test is another crate
   (a separate test crate): `config.coverage.filter = "mycrate".into();` or
   `EPHYRA_COVERAGE_FILTER=mycrate`. Use the crate name with underscores, or
   a module path (`mycrate::parser`); it covers the crate's methods too, so
   don't copy the `<...>` of demangled method names. `#[ephyra::test]`
   defaults to the test crate's own name, which is right for tests inside
   the project and wrong for a separate test crate: check the `coverage:`
   line of the first report names the crate you meant.

Run with one test thread (`RUST_TEST_THREADS=1`): counters are
process-global. For a file and line view, set `EPHYRA_COVERAGE_HTML=dir` (or
use `ephyra test`, which writes `.ephyra/coverage/<property>.html`). The
report then shows `coverage: 3 of 5 functions in `tlv`
reached`, `never reached: ...` and `partly covered: ...` (in code,
`report.coverage`). Add targets to steer at unreached code:
`config.coverage.targets.push(ephyra::config::CoverageTarget::new("src/parser.rs", 120..=140))`,
or one JSON line per target in `.ephyra/targets.jsonl`. Fault localization
runs on a failure with coverage on and adds `suspects` to the report
(`report.locate`).

## The `ephyra` command

`cargo install --git https://github.com/datadog-labs/ephyra ephyra-cli`.

| Command | Does |
|---|---|
| `ephyra guide` | Prints the guide to writing generators and strategies that pass `check` |
| `ephyra check` | Verifies every `#[ephyra::generator]` and every strategy of a property that can discard: values satisfy the predicate, shrink candidates and mutants stay valid, shrink chains settle, metrics are finite. Exit 0 passed, 1 failed, 2 work left. `--list` shows what it found. |
| `ephyra test` | Runs the suite with artifacts in `.ephyra/` |
| `ephyra visualize` | A local page for exploring and rerunning runs |

The CLI instruments builds by default; pass `--no-instrument`, or set
`LLVM_PROFILE_FILE`, if that gets in the way.

## Things that do not exist

- `gens::vec_of`, `gens::vec_of_len`, `gens::tuple_vec`, `gens::bool_`.
- Integer generators for `i128` or `u128`.
- A per-test timeout, or process isolation.
- A public `Report::new`.
- A driver for subprocesses or servers: write it (spawn, feed input, compare
  output, with a timeout).
- Automatic seeding from files: read them and `pool.invest(...)` them.
