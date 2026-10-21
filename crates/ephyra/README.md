# ephyra

Property-based testing for Rust, built for tests that are **written and
verified by coding agents**: you author the generator, the support
predicate, the shrinker and the mutator; the library verifies all of them
and makes every failure reproducible from its seed.

```rust
use ephyra::prelude::*;

#[ephyra::test]
fn sorting_keeps_every_element(mut v: Vec<i32>) -> bool {
    let mut expected = v.clone();
    expected.sort();
    v.sort();
    v == expected
}
```

A failure prints everything needed to act on it:

```text
ephyra: FAILED, property returned false
  counterexample: [0, -1]
  original:       [1000, -268, -1000, 341, 0] (size 5, iteration 13)
  shrinking: 6 of 41 candidates accepted in 0.4 ms
  after 13 passing tests (0 discarded) in 1.4 ms
  reproduce: EPHYRA_REPLAY=13538456636133602880:13
  or rerun the whole search: Config::default().seed(13538456636133602880)
  remembered in .ephyra/regressions/insertion_sort.jsonl
```

## The idea

Most property-testing libraries hide shrinking inside the generator so that
nobody has to write a shrinker. ephyra takes the opposite position:
shrinkers and mutators are ordinary code that an agent can write in a
minute, and their correctness is a property the library checks for you.

- **Generators are explicit and uniform.** `Source` is a seeded random
  source with uniform primitives. Edge-case bias is composed from visible
  generators (`gens::edges`, `gens::f64_bits`, ...), and `any::<T>()` is a
  documented `frequency` mix you can rebuild or replace.
- **Every generator owns its shrinker and mutator.** Built-ins carry hooks
  that stay in their range. For your own generators, attach them with
  `with_shrink` / `with_mutate`, use a bijection with `iso`, or derive
  structural hooks with `#[derive(Arbitrary)]`. Without a mutator, a
  generator mutates by drawing a fresh value.
- **Correctness is verifiable.** `check_shrinker` checks that every
  candidate stays in the generator's support and is strictly smaller;
  `check_mutator` checks mutants; `check_generator` checks the support
  itself. They run through `check`, so they shrink and replay like any
  other property.
- **Everything is reproducible.** A failure is `seed:iteration`;
  `EPHYRA_REPLAY` regenerates it without searching. Failures are
  remembered in a regressions file that is replayed first next time and
  flags entries whose generator has changed.
- **Everything is configurable and machine-readable.** Every setting is a
  `Config` field, a `EPHYRA_*` variable, or a JSON file, and `Report`
  is JSON as well as text.
- **Targeted search.** Call `target(score)` inside a property and the
  runner mutates the best-scoring inputs with the generator's mutator to
  push the score up.

## Writing generators

```rust
use ephyra::{gens, Gen, GenExt, Source, check_shrinker, Config};

#[derive(Debug, Clone, PartialEq)]
enum Tree { Leaf, Node(Box<Tree>, i32, Box<Tree>) }

// Valid by construction: each key is drawn from the range its position allows.
fn bst(g: &mut Source, lo: i32, hi: i32, depth: usize) -> Tree {
    if depth == 0 || lo > hi || !g.bool_with(0.8) {
        return Tree::Leaf;
    }
    let key = g.int(lo..=hi);
    Tree::Node(Box::new(bst(g, lo, key - 1, depth - 1)), key, Box::new(bst(g, key + 1, hi, depth - 1)))
}

// Write an ordinary eager shrinker. The macro compiles it into the visitor API,
// which stops producing candidates as soon as the runner accepts one.
#[ephyra::shrinker]
fn shrink_bst(t: &Tree) -> Vec<Tree> {
    match t {
        Tree::Leaf => Vec::new(),
        Tree::Node(l, k, r) => {
            let mut out = vec![(**l).clone(), (**r).clone(), Tree::Leaf];
            out.extend(shrink_bst(l).into_iter().map(|l| Tree::Node(Box::new(l), *k, r.clone())));
            out.extend(shrink_bst(r).into_iter().map(|r| Tree::Node(l.clone(), *k, Box::new(r))));
            out
        }
    }
}

fn keys(t: &Tree, out: &mut Vec<i32>) { if let Tree::Node(l, k, r) = t { keys(l, out); out.push(*k); keys(r, out); } }
fn is_bst(t: &Tree) -> bool { let mut v = Vec::new(); keys(t, &mut v); v.windows(2).all(|w| w[0] < w[1]) }
fn size(t: &Tree) -> u64 { match t { Tree::Leaf => 0, Tree::Node(l, _, r) => 1 + size(l) + size(r) } }

let trees = (|g: &mut Source| bst(g, -100, 100, 6)).with_shrink(shrink_bst).boxed();
check_shrinker(&Config::default(), trees.clone(), is_bst, size).assert_passed();
```

`#[ephyra::shrinker]` accepts a deliberately small language for building
one `Vec<T>` of candidates, streamed in their original order; its
documentation lists what it accepts and rejects. A shrinker written against
the visitor directly needs no macro (see `Gen::shrink`).

`#[derive(Arbitrary)]` covers plain structs and enums, with
`#[ephyra(gen = ...)]` per field and `#[ephyra(support = ...)]` per
type; `any::<T>()` gives the default generator of any std type.

A `#[ephyra::test]` property takes a strategy with
`#[ephyra(gen = <expression>)]` on a parameter, or `gen = <expression>`
for its whole input, which can keep relations the property `assume!`s.
`#[ephyra::generator]` marks a predicate `fn(&T) -> bool`, or a generator,
optionally naming the predicate it serves and a `metrics` function;
`ephyra check` finds them and checks every generator, and every strategy
of a property that can discard, over thousands of inputs.

## Checking properties

```rust
use ephyra::{check, gens, Config, TestResult, classify, target};

let report = check(&Config::default().tests(500), gens::vec(gens::int(-50i32..=50)), |v| {
    classify(v.is_empty(), "empty");
    target(v.len() as f64);           // optional: steer toward long inputs
    let sum: i32 = v.iter().sum();
    TestResult::check(sum.abs() <= 50 * v.len() as i32, "sum out of bounds")
});
report.assert_passed();
```

Properties may return `bool`, `()`, `Result<_, E: Debug>` or `TestResult`;
a panic is a failure with its message and location captured. `assume!`
discards inputs that do not apply.

## Writing your own loop

`check` is a loop over public steps, and the steps are yours to arrange.
A `Run` applies the configuration, owns the seed and the report, and offers
`draw`, `test`, `keep`, `pool_pick`, `mutate` and `wants_more`; `finish`
shrinks the first failure, localizes it, remembers it and writes the
report. The `Run` documentation spells out each step, and
[`examples/custom_loop.rs`](https://github.com/datadog-labs/ephyra/blob/main/crates/ephyra/examples/custom_loop.rs)
replaces the built-in targeted phase with a hill-climb.

Shrinking is a step as well. `run.shrink()` runs the built-in driver on
the failure whenever the loop likes, and `run.reproduces(&candidate)`
asks the run's oracle whether a candidate fails the same way, so a
shrinking strategy of your own is a loop over candidates ending in
`run.shrunk_to(minimal)`:

```rust
let mut current = run.failure().unwrap().clone();
'outer: loop {
    for candidate in run.generator().shrink_candidates(&current) {
        if run.reproduces(&candidate) { current = candidate; continue 'outer; }
    }
    break;
}
run.shrunk_to(current);
```

`finish` reports whatever the loop settled on, and only shrinks itself
when the loop did nothing.

Three loops ship in `ephyra::loops`, after the loops of Etna's
PropLang: `fuzz` (feedback: coverage novelty), `target` (the property's
`target` score) and `perf_fuzz` (how long the property took), each over a
seed pool policy of your choice; the module documentation lists the pools
and `loops::localize`. See
[`examples/shipped_loops.rs`](https://github.com/datadog-labs/ephyra/blob/main/crates/ephyra/examples/shipped_loops.rs)
and
[`examples/perf_fuzz.rs`](https://github.com/datadog-labs/ephyra/blob/main/crates/ephyra/examples/perf_fuzz.rs).

For stateful tests, whose inputs are traces of steps, `loops::causality`
explains a failure the way Antithesis's causality analysis does: it
rewinds the failing trace to earlier prefixes, continues each many times
with fresh randomness, and reports how often the failure happens again
after each prefix. A jump in that rate marks the step after which the
failure became likely, which matters most when the step that caused a
failure and the step that noticed it are far apart. The generator
implements `Trace` (a trace's length, and how to resume one after a
prefix); the curve is in the report and in `ephyra visualize`.

`#[ephyra::test]` can install one of these loops, or a loop of your own,
without giving up the test harness integration:

```rust
#[ephyra::test(
    tests = 1_000,
    search = |run| {
        let mut pool = ephyra::loops::Fifo::new();
        ephyra::loops::fuzz(run, &mut pool, 1_000);
    }
)]
fn parser_never_panics(input: String) {
    let _ = parse(&input);
}
```

The macro calls `finish` afterward, so shrinking, regressions and reports
work as with the default search.

## Reproducing and steering runs

| Variable | Effect |
|---|---|
| `EPHYRA_REPLAY=seed:iteration` | regenerate and test one input first |
| `EPHYRA_SEED=N` | rerun the same search |
| `EPHYRA_TESTS=N`, `EPHYRA_BUDGET_MS=N`, `EPHYRA_MAX_SIZE=N` | change the budget |
| `EPHYRA_SHRINK=0` | report the original failing input |
| `EPHYRA_TARGET_TESTS=N` | budget of the targeted phase |
| `EPHYRA_REPORT_JSON=path` | append every report as a JSON line |
| `EPHYRA_OBSERVATIONS=dir` | write per-case observation traces |
| `EPHYRA_CONFIG=path.json` | any subset of `Config` fields |

The full list is in the `config` module documentation.

## Features

- `json` (default): JSON reports and config, the regressions file.
- `derive` (default): `#[derive(Arbitrary)]` and `#[ephyra::test]`.
- `coverage`: in-process coverage for binaries built with
  `RUSTFLAGS="-C instrument-coverage"`: per-property coverage in the
  report, coverage-guided search, fault localization (suspect functions
  ranked from the coverage of passing and failing runs), coverage targets
  the search steers toward, and a coverage grid page per property.
- `coverage-force`: assume the build is instrumented when the build script
  cannot see the flag (profile rustflags, `cargo rustc -- …`). An
  uninstrumented binary then fails to link, by design.

Uninstrumented builds compile the coverage API to no-ops. The `coverage`
module documentation covers setup (`RUST_TEST_THREADS=1`,
`LLVM_PROFILE_FILE`, nightly branch coverage), targets and the grid page.

## Command-line workflow

The `ephyra` command is in the
[`ephyra-cli`](https://github.com/datadog-labs/ephyra/tree/main/crates/ephyra-cli)
package. `ephyra test` runs a suite into one artifact directory;
`ephyra check` finds the `#[ephyra::generator]` predicates that still
need a generator and checks every generator and `gen =` strategy over
thousands of inputs; `ephyra visualize` serves the results for exploring
and rerunning.

## Observation traces

Set `EPHYRA_OBSERVATIONS=.ephyra/observed` to write one observation
file per property in the OpenPBTStats format, and open it in
[Tyche](https://github.com/tyche-pbt/tyche-extension) to see the sample
breakdown, feature distributions, coverage and timing charts. A property can
add trace-only values without affecting ordinary runs:

```rust
ephyra::observe("expression nodes", || expression.size());
let checked = ephyra::measure("type checking", || type_check(&expression));
```

Observation-value closures are evaluated only while a trace is enabled;
measured operations always run, but incur timing overhead only for traces.

The observation file is one sink. Implement `ephyra::sink::Sink` and call
`ephyra::sink::install` once per process to send every evaluated input, and
then the report, somewhere else as well, such as a metrics agent. A sink's
`open` sees each run's property name and can return `None` to skip it.

Minimum supported Rust version: 1.88.

## Status

The API is not yet frozen.

Planned: PerfFuzz proper, where inputs that set a new maximum execution
count on any counter are kept and the total path length is the score
when the property reports none, with the hottest input and functions in
the report. Today `loops::perf_fuzz` climbs the property's measured
time, and coverage-guided search rewards new code only (AFL hit-count
classes saturate at 128).

## License

MIT.
