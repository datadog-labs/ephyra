# Writing generators and strategies for ephyra

This guide is for whoever writes the Rust generators and strategies that
ephyra properties need, usually a coding agent. `ephyra check` finds what
is missing and checks what is written; `ephyra guide` prints this text.

## What to write

Everything is ordinary Rust in the project, next to the code it serves.

```rust
use ephyra::{gens, Gen, GenExt, TestResult};

// A generator whose every value is valid: no predicate. Its metrics describe
// what it produces, once, for every property that uses it.
#[ephyra::generator(metrics = word_metrics)]
fn words() -> impl Gen<Value = String> {
    gens::string_len(gens::char(), 0..80)
}

fn word_metrics(word: &String, _size: usize) -> Vec<(&'static str, f64)> {
    vec![("bytes", word.len() as f64)]
}

// A property keeps its assertions and returns `()`.
#[ephyra::test(gen = words())]
fn bytes_roundtrip(word: String) {
    let bytes = word.clone().into_bytes();
    assert_eq!(String::from_utf8(bytes).unwrap(), word);
}

// One parameter with a strategy of its own; the other keeps its default.
#[ephyra::test]
fn repeat_multiplies_length(count: u8, #[ephyra(gen = words())] word: String) {
    assert_eq!(word.repeat(usize::from(count)).len(), word.len() * usize::from(count));
}

// A predicate: the values a generator must produce. Kept as it is.
#[ephyra::generator]
fn sorted(values: &[u8]) -> bool {
    values.is_sorted()
}

// A generator for it, with metrics describing what it produces.
#[ephyra::generator(sorted, metrics = sorted_metrics)]
fn sorted_bytes() -> impl Gen<Value = Vec<u8>> {
    gens::vec(gens::int(0u8..=9)).map(|mut v| {
        v.sort();
        v
    })
}

fn sorted_metrics(values: &Vec<u8>, size: usize) -> Vec<(&'static str, f64)> {
    vec![("len", values.len() as f64), ("len per size", values.len() as f64 / (size + 1) as f64)]
}

// A strategy: a generator of a property's whole input, so it can keep the
// relation the property assumes.
fn ordered_pairs() -> impl Gen<Value = (u8, u8)> {
    gens::dependent(gens::int(0u8..=254), |lo| gens::int(*lo + 1..=255))
}

#[ephyra::test(gen = ordered_pairs())]
fn midpoints_lie_between(lo: u8, hi: u8) -> TestResult {
    ephyra::assume!(lo < hi);
    let mid = lo + (hi - lo) / 2;
    TestResult::check(lo <= mid && mid < hi, "the midpoint left the range")
}

fn main() {}
```

- A generator is `#[ephyra::generator(<predicate>, metrics = <fn>)]` on a
  function with no parameters returning `impl Gen<Value = T>` or `BoxGen<T>`.
  Both arguments are optional. Name a predicate only when some values of `T`
  are invalid; a generator whose every value is valid takes a bare
  `#[ephyra::generator]`, and a predicate that just returns `true` is noise
  (`ephyra check` warns about one). The predicate is called with `&T`, so
  a predicate over `&[E]` or `&str` accepts a generator of `Vec<E>` or
  `String`.
- A predicate is `#[ephyra::generator]` on `fn(&T) -> bool`. `ephyra check`
  lists every predicate that no generator names as `needs a generator`.
- A strategy replaces a property's `Arbitrary` inputs: `#[ephyra(gen =
  <expression>)]` on one parameter, or `gen = <expression>` on
  `#[ephyra::test]` for all of them (a generator of the parameter's type
  when there is one, else of the tuple of all parameters in order, which can
  keep relations between them).
- `metrics` is `fn(&T, usize) -> Vec<(&'static str, f64)>`: numbers about one
  value and the size it was drawn at (see [Metrics](#metrics)). They describe
  a generator, so put them on the generator. `metrics = <fn>` on a property is
  only for a strategy of a property that can discard (see below).

Keep predicates and properties unchanged: the predicate defines which values
are valid, and a property's `assume!` defines which inputs apply. Do not
constrain a strategy to make a property pass: a failure on an applicable input
is a counterexample, not a reason to exclude that input.

## Writing properties

A property may return `()`, `bool`, `Result<_, E: Debug>` or `TestResult`.
Prefer `()` with `assert!` and `assert_eq!`: a panic is a failure with its
message and location in the report, `assert_eq!` shows both values, and
shrinking keeps the failure at the same assertion. Return `TestResult` only to
discard inputs that do not apply, with `ephyra::assume!(condition)`.

Porting from proptest or quickcheck, keep the bodies: `prop_assert!` and
`prop_assert_eq!` become `assert!` and `assert_eq!`, `prop_assume!` becomes
`ephyra::assume!` (returning `TestResult`), `#[strategy(s)]` on a parameter
becomes `#[ephyra(gen = s)]`, and parameters without one keep their
`Arbitrary` defaults.

For targeted search, call `ephyra::target(score)` inside the property:
after random testing, the runner mutates the best-scoring inputs to push the
score up, with no loop to write. `search = |run| ephyra::loops::target(run,
&mut ephyra::loops::Heap::new(), 1_000)` on `#[ephyra::test]` installs
a shipped loop instead (`loops::fuzz` climbs coverage). Hand-written `Run`
loops are rarely needed.

## What `ephyra check` verifies

Each generator gets an ignored `__ephyra_check_<name>` test, and so does
each property with a strategy that can discard (it returns `TestResult`). A
property returning `()`, `bool` or `Result` never discards, so its strategy
has nothing to check beyond its generators' own checks. `ephyra check` runs
these tests over thousands of inputs as full ephyra runs, with coverage,
reports and a counterexample when one fails. Per input:

- A generator's value satisfies its predicate. A strategy's value may be
  discarded by the property (it is counted, and too many make the run give up).
- The first 64 shrink candidates differ from the value and stay valid
  (satisfy the predicate, or are not discarded by the property).
- Following first candidates from the value stays valid, never revisits a
  value, and settles within 1000 steps.
- Four mutants are drawn without a discard and stay valid.
- Every metric is finite; each is reported with its distribution.

A failing check shrinks to a small value whose hooks misbehave. Run
`ephyra check` again after every change; `ephyra check NAME` checks one.

## Running the check

Run it from the package or workspace directory:

- `ephyra check --list` reads the source without building and prints every
  annotated function, then the work left: `needs a generator: <predicate>`
  for each predicate no generator names, and `unresolved: …` for a generator
  whose predicate name matches none (or several).
- `ephyra check` builds each test target once and runs every check. Each
  prints its ephyra report (on failure: the problem, the counterexample and
  a replay line), and a summary of `passed` / `FAILED` lines and remaining
  work ends the output.
- `ephyra check NAME…` runs only those checks; `--tests N` changes the
  inputs per check (smaller while iterating, the default 10,000 to finish);
  arguments after `--` go to cargo, e.g. `-- --features coverage` for coverage
  when the crate enables ephyra's `coverage` feature through one.

The exit code is 0 when every check passed and nothing is left, 2 when the
checks passed but work is left, and 1 when a check failed or did not build (a
build failure prints the compiler's messages). Reports, observation traces
and coverage pages go to `.ephyra/`.

## Final-type components

Conceptually generate `Source -> T`, shrink `&T -> sequence<T>`, and mutate
`(&T, Source) -> T`. All three operate on the FINAL output type and preserve
its validity. The runner, not the shrinker, decides which shrink candidates
still reproduce a property failure.

`Gen: Send + Sync` has these methods (the actual shrink sequence is lazy):

```text
type Value = T;
fn generate(&self, g: &mut ephyra::Source) -> T;
fn shrink(&self, value: &T,
    visit: &mut dyn FnMut(T) -> core::ops::ControlFlow<()>)
    -> core::ops::ControlFlow<()>;
fn mutate(&self, value: &T, g: &mut ephyra::Source) -> T;
```

A `Fn(&mut Source) -> T + Send + Sync` closure supplies generation. Default
shrink emits nothing and default mutation generates a fresh value. Mutation
ALWAYS returns `T`, not `Option<T>`; it may return the unchanged input when no
local change applies, or explicitly fall back to generation. Provide useful
final-type hooks for nontrivial domains; if none are possible, state that
limitation in a Rust comment rather than pretending they exist. Shrinkers must
respect `visit` returning `Break`, terminate, and make progress under a stated
well-founded measure. Do not emit unchanged candidates or introduce cycles. Use
`Source` for randomness. Bound recursion and rejection, and guard arithmetic,
empty ranges and empty collections.

Attach hooks AFTER the last lossy transformation, or implement `Gen` directly.
A generic assembly pattern:

```rust
use ephyra::{Gen, GenExt, Source};
use core::ops::ControlFlow;

fn components<T, G, S, M>(generate: G, shrink: S, mutate: M) -> impl Gen<Value = T>
where
    G: Fn(&mut Source) -> T + Send + Sync,
    S: Fn(&T, &mut dyn FnMut(T) -> ControlFlow<()>) -> ControlFlow<()> + Send + Sync,
    M: Fn(&T, &mut Source) -> T + Send + Sync,
{
    generate.with_shrink(shrink).with_mutate(mutate)
}

let generator = components(
    |source: &mut Source| source.int(0u8..=255),
    |value: &u8, visit: &mut dyn FnMut(u8) -> ControlFlow<()>| {
        if *value > 0 { visit(*value / 2)?; }
        ControlFlow::Continue(())
    },
    |value: &u8, _source: &mut Source| *value ^ 1,
);
assert_eq!(generator.shrink_candidates(&8), vec![4]);
assert_eq!(generator.mutate(&8, &mut Source::new(1, 8)), 9);
```

Combinator semantics:

- `.map(f)` and `.and_then(f)` DROP custom shrinking and mutation, falling back
  to fresh generation for mutation. In particular,
  `.with_shrink(...).map(...)` does not give the final output a shrinker.
- `.iso(forward, back)` transports hooks through a bijection. Do not invent a
  fake inverse (e.g. always returning an empty vector). Sorting, deduplication,
  merging and collection into a map generally lose information; use final-type
  hooks instead of claiming these are bijections.
- `.dependent(|outer: &A| inner_generator)` returns `(A, B)`, not just `B`,
  and preserves the joint relationship. `A` must be `Clone`.
- `.filter("reason", |value: &T| ...)` filters generation, shrinking and
  mutation. Mutation retries then falls back to filtered generation; both can
  discard after a finite retry limit. Prefer direct valid construction when
  rejection is expensive.
- `.with_shrink(...)` visits custom candidates before the inner candidates;
  `.with_mutate(...)` REPLACES the inner mutator. There is no implicit fallback.
- Tuples combine component generators and hooks. Use a joint generator when
  fields have relational constraints; independently valid fields can form an
  invalid tuple.

## API examples

Bounds are illustrative. `Vec` element values must be `Clone`.

```rust
use ephyra::{gens, BoxGen, Gen, GenExt, Source};

let _integer = gens::int::<u64>(..); // full-width, uniform
let _edges = gens::edges(0u8..=255);
let _coin = gens::bool();
let _biased = gens::bool_with(0.25);
let _chars = gens::char(); // Unicode scalars, with ASCII bias; not a category filter
let _vector = gens::vec(gens::int(0u8..=255)).len(0..=32);
let _string = gens::string_len(gens::char(), 0..=32);
let _by_size = gens::sized(|size| gens::vec(gens::int(0u8..=255)).len(0..=size));
let _choice: BoxGen<u8> = gens::one_of(vec![
    gens::constant(0u8).boxed(), gens::int(1u8..=255).boxed(),
]).boxed(); // one_of returns Frequency, not BoxGen
let _weighted = gens::frequency(vec![
    (1, gens::constant(0u8).boxed()), (3, gens::int(1u8..=255).boxed()),
]);
let dependent = gens::int(1u8..=10).dependent(|hi: &u8| gens::int(0u8..=*hi));
let even = gens::int(0u8..=254).filter("even", |n: &u8| *n % 2 == 0);
let wrapped = even.iso(|n| (n,), |value: &(u8,)| value.0);
assert!(!wrapped.shrink_candidates(&(8,)).is_empty());
let mapped = gens::int(0u8..=255).map(|n| (n,));
assert!(mapped.shrink_candidates(&(8,)).is_empty()); // intentional hook loss

let mut source = Source::new(1, 8); // runner supplies Source in real generation
let _size: usize = source.size();
let _value: u64 = source.int(..);
let _index: usize = source.index(3); // requires a nonempty collection
let _picked: &u8 = source.pick(&[1u8, 2, 3]); // requires nonempty slice
let _boolean: bool = source.bool();
let _biased_boolean: bool = source.bool_with(0.25);
let (hi, lo) = source.draw(&dependent);
assert!(lo <= hi);
```

There is no `gens::vec_of`, `gens::vec_of_len`, `gens::tuple_vec`, or
`gens::bool_`. `gens::vec` takes ONE element generator; choose lengths with
`.len(range)`. `GenExt`, not `Gen`, defines combinator methods. `with_shrink`
takes a visitor callback, not a function returning `Vec<T>`
(`#[ephyra::shrinker]` converts one that does).

`gens::vec(element)` defaults to a size-dependent maximum. An explicit finite
`.len(0..=N)` replaces that maximum with N: it does NOT grow with the size.
`string_len` has the same rule. Distinguish size scheduling from domain
restrictions: do not silently cap all sizes to a small fixed subset. For rare
but important boundaries or equal pairs, deliberate sampling may be useful;
keep broader support rather than replacing it with a finite palette, and
explain any size/support tradeoff in comments.

## Metrics

A metrics function sees one freshly generated value and the size it was drawn
at; it does not see construction history, shrinking, mutation or coverage.
Prefer a small, nonredundant set covering meaningful shapes, boundaries,
degeneracy and the response to size.

Each metric must be finite for every valid value, including empty and
singleton values and size zero; a non-finite one fails the check. Keep them
pure: no IO, randomness, assertions or panics. Guard division, indexing,
integer arithmetic and logarithms, and use 0.0/1.0 for indicators. Say in a
comment what each counts, what its denominator is, and what low and high
values mean:

- A final value cannot reveal its history: a non-overlapping map's segment
  count is not its insertion count.
- Distinct values, adjacent changes, equal pairs and duplicated groups are
  different quantities; match names to formulas.
- Zero observed boundaries means "not observed", not "unreachable".
- Raw counts against size show the response to size; include raw counts
  rather than hiding them behind ratios or caps.
- Invariants expected always true are sanity checks, not diversity metrics.
