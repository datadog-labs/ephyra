# ephyra-cli

The `ephyra` command for [ephyra](../ephyra) property suites. It is
artifact-first: one command runs the tests and records everything, one checks
the generators and strategies the suite relies on, and another opens the
local visualization and rerun loop.

```sh
cargo install ephyra-cli    # or, in this repository: cargo run -p ephyra-cli --
ephyra test --features coverage
ephyra check -- --features coverage
ephyra visualize
```

`ephyra test` routes reports, OpenPBTStats observation traces, coverage
pages, targets, profraw images and regressions into `.ephyra` and uses an
isolated instrumented Cargo target directory. Pass `--no-instrument` for the
ordinary build, `--dir` to choose another artifact root, or `--` before test
harness arguments.

`ephyra visualize` opens a local page over the artifact root (grid pages,
latest reports, targets, discovered tests) that discovers and runs any
property or test on request with `cargo` on an instrumented build, streams the
output, and refreshes the pages as they are written; served grid pages gain
"Run" and "Add to targets.jsonl" buttons.

## Generators and strategies

Generators and strategies are ordinary Rust in the project:

```rust,ignore
#[ephyra::generator]
fn sorted(values: &[u8]) -> bool {
    values.is_sorted()
}

#[ephyra::generator(sorted, metrics = sorted_metrics)]
fn sorted_bytes() -> impl Gen<Value = Vec<u8>> { /* ... */ }

#[ephyra::test(gen = ordered_pairs(), metrics = gap)]
fn midpoints_lie_between(lo: u8, hi: u8) -> TestResult {
    ephyra::assume!(lo < hi);
    /* ... */
}
```

- `#[ephyra::generator]` on `fn(&T) -> bool` marks a predicate: the values a
  generator must produce.
- `#[ephyra::generator(<predicate>, metrics = <fn>)]` on a function with no
  parameters marks a generator; both arguments are optional, and a generator
  whose every value is valid needs no predicate.
- `#[ephyra(gen = <expression>)]` on a parameter, or `gen = <expression>`
  on `#[ephyra::test]` for the whole input, gives a property a strategy;
  a whole-input strategy can keep relations such as `lo < hi` that the
  property `assume!`s.
- `metrics` is a `fn(&T, usize) -> Vec<(&'static str, f64)>` of a value and the
  size it was drawn at, reported with each check; it belongs on the generator.

The macros add an ignored `__ephyra_check_<name>` test for every generator,
and for every strategy of a property that can discard (it returns
`TestResult`); a property returning `()`, `bool` or `Result` never discards,
so its generators' own checks cover its strategy. Plain `cargo test` never
runs these tests.

## `ephyra check`

`ephyra check` reads the workspace's annotated functions from source (with
`syn`, following `mod` declarations from every library and integration-test
target; no build, no macro expansion). It lists them, pairs each generator
with the predicate it names, and then builds each test target once and runs
every check test (or those of the functions named on the command line) with
the same artifact routing and instrumentation as `ephyra test`.

A check is a full ephyra run over thousands of inputs (10,000 unless
`--tests N`), with a report, coverage page and a shrunk counterexample on
failure. For every input:

- a generator's value satisfies its predicate; a strategy's value may be
  discarded by the property's `assume!` (counted, and too many give up);
- the first 64 shrink candidates, every step of the first-candidate shrink
  chain, and four mutants stay valid, and the chain settles;
- every metric is finite, and each is reported with its distribution.

It ends with a summary: passed and failed checks, predicates that no
generator names (`needs a generator`), and generators naming a predicate that
does not resolve. It exits 1 when a check failed, 2 when only such work is
left, and 0 otherwise. `--list` prints what the index found without building.
Arguments after `--` go to cargo, e.g. `-- --features coverage`.

The index is syntactic: a generator's predicate is resolved by the last
segment of its path within the package, and an ambiguous or unknown name is
reported, not guessed. `cfg` is not evaluated, and functions a macro
generates are not seen. `--list` warns about predicates that just return
`true`.

## Writing them with an agent

`ephyra guide` prints a reference for whoever writes the generators, usually
a coding agent: the annotations, what a check verifies, the `Gen` hooks and
combinators, and how to write metrics. The loop is: `ephyra check --list`
for what is missing, write the Rust, `ephyra check` until it passes.
[`examples/synthesis`](../../examples/synthesis/README.md) has three small
projects with a predicate each and no generator yet.

## License

MIT.
