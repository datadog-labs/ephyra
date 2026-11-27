# Changelog

All notable changes to `ephyra` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

- `gens::one_of` and `gens::frequency` are combinators, like `map`: a value
  does not say which alternative produced it, so a choice has no shrinker,
  and mutation generates a fresh value from a fresh choice. They used to run
  every alternative's shrinker, and a random alternative's mutator, on every
  value, which gave values no alternative can produce (`(6, [4 bytes])` from
  a v4/v6 address choice), false failures under `target`, and aborted runs
  (described in #70). For alternatives that should shrink, give the value its
  alternative: an enum with `#[derive(Arbitrary)]`, or `dependent` over an
  index. A `recursive` generator whose top level is a choice has no shrinker
  either.
- `any::<integer>()`, `any::<f32>()` and `any::<f64>()` keep their mix of
  values but no longer get their hooks through a choice: they shrink and
  mutate with the hooks of the full range (integers) and of the bit patterns
  (floats), which hold for every value of the type.
- `gens::dependent` mutates by mutating the first value and generating the
  second afresh from it; it no longer mutates the second value in place.
- `ShrinkStats::rejected_not_smaller` and its field in the JSON report are
  gone, with the rule they counted.
- A checker's counterexample is shrunk only when that cannot run away with a
  broken shrinker: `check_shrinker` shrinks under the caller's `size` (a
  failing value larger than one already found is discarded), and the checks
  behind `ephyra check` report the generated value that exposed the problem,
  unshrunk.
- A property that returns `Result<T, E>` now requires `T: Testable` and takes
  the verdict of the `Ok` value: `Ok(false)`, `Ok(TestResult::Fail(..))` and
  `Ok(Err(..))` fail, `Ok(TestResult::Discard(..))` discards. It used to pass on
  every `Ok`. An `Ok` type that is no verdict no longer compiles. By Bruno
  Volpato (#63).

### Fixed

- `gens::f32()` could generate and mutate to `1.0`, outside its documented
  `[0, 1)` (#40). Fix by Yusuf Eren Bozkurt (#137).
- `gens::f64_range` returned `hi` on every draw when `hi - lo` overflows
  (`f64_range(f64::MIN..=f64::MAX)`), and its mutation step went to a bound
  (#41). Fix by Bruno Volpato (#69).
- Properties whose names differ only in punctuation or non-ASCII letters
  shared a regressions file and replayed each other's saved failures (#48).
  Fix by Yusuf Eren Bozkurt (#136).
- Saved failures over `HashSet` and `HashMap` inputs were skipped as stale,
  so the next run could pass (#46). Fix by Bruno Volpato (#73).
- A regressions file lost entries when its last line had no newline (the
  next entry was glued onto it), and loaded as empty when one line was not
  UTF-8 (#59, two of its three causes).
- A run whose every input was discarded passed when a time budget, a zero
  budget or a loop ended it before the discard limit; it now gives up. By
  Bruno Volpato (#65).
- A panic in code after a `filter` that gave up aborted the run; the input
  now counts as discarded. By Bruno Volpato (#76).
- `Config::failures(n)` never ended when every input failed with fewer than
  `n` signatures; failing inputs now count toward `Config::tests` (#42). By
  Bruno Volpato (#66).
- The targeted phase ignored the run's discard limit, so a property that
  called `target` could pass with many times `max_discard_ratio * tests`
  discards (#102).
- The shrink driver skipped every candidate that printed longer than the
  failing input, so a bool never shrank; float shrinking stops at one. By
  Bruno Volpato (#71). The generator alone now decides what is simpler: the
  driver has no rule on a candidate's rendering at all, and the step and time
  budgets end every descent. Together with the reordered specials,
  `any::<f64>()` no longer offers 1.0 for 0.5 (#92).
- A panic in user code while a failure was shrunk escaped `check` and lost
  the failure. By Bruno Volpato (#72).
- A failure raised with `resume_unwind` was reported with an earlier handled
  panic's message and marked FLAKY (#58), or as `<non-string panic payload>`
  (#101).
- Two panics on one line were one failure to the shrinker, which could trade
  the failure found for the other one; the signature now has the column
  (#100).
- Replaying a point where the generator panicked passed (#97), and a replay
  with no seed reported a random seed instead of the point's (#98).
- `check_shrinker` and `check_mutator` could report a counterexample outside
  the support they were given (#113).
- `gens::dependent` could shrink to a pair outside its joint support when the
  inner generator filters: a regenerated second value whose filter ran out of
  attempts is no longer a candidate (#44).
- `gens::f32_bits()` shrank an infinity to itself; it now shrinks to
  `f32::MAX` / `f32::MIN` (#47).
- `any::<f32>()` and `any::<f64>()` shrank in cycles (`0.5 -> 1.0 -> 0.5`,
  `MAX -> inf -> MAX`): the special values are now listed in the order the
  float shrinker reduces.

### Added

- `tests/gen_contract.rs`: one property, every built-in generator keeps its
  contract (replay, support under generate, mutate and shrink, shrinking that
  settles, generation that spreads over the support), over generated
  arguments. It catches each of the generator bugs above.
- `tests/replay_contract.rs`: one property, a saved failure is replayed for
  its own property only: the regressions store against a model under
  generated names, saves and file edits, and a save-then-replay round trip
  for every kind of input. It catches each of the replay bugs above.
- `tests/verdict_contract.rs`: one property, a run's verdict matches what its
  inputs did: scripted properties in every return shape, under drawn settings
  (tests, discard ratio, `failures`, a zero budget, `target`, a filter that
  gives up), must end, fail if and only if an input failed, pass only on
  evidence, and never abort on a discard. It catches each of the runner bugs
  above.
- `tests/report_contract.rs`: one property, what a failing report says is
  true: its message is what the property does on the counterexample, which
  fails the way the found input did; its replay point and seed reproduce it;
  the checkers' counterexamples lie in their support. It catches each of the
  report bugs above.
- `tests/gen_contract.rs` covers `one_of` over structured alternatives.
- `regressions/`: every fixed bug as a reversed patch, with the properties
  that must catch it; CI checks fix and bug soundness on every pull request.
  CONTRIBUTING.md describes the method.

## [0.0.1] - 2026-10-29

The first release on crates.io.

### Added

- Coverage filter usability: `Config::coverage.filter`
  (`EPHYRA_COVERAGE_FILTER`) ignores surrounding whitespace, a leading `<`
  and a trailing `>`, and reads `>::` as `::`, so `<my_lib::Parser>`,
  `<my_lib::Parser` and `<my_lib::Parser>::parse` select what
  `my_lib::Parser` and `my_lib::Parser::parse` do; a type filter also
  covers its trait methods (`<my_lib::Parser as Trait>::m`). A filter that
  selects no functions (even when the property's own function is still
  tracked), or a crate filter that selects fewer than five, adds a
  `coverage_filter` health warning that says how to set the filter and
  notes when it is the property's own crate, the default that leaves a
  separate test crate covering only itself.

- Many distinct failures per run: `Config::failures(n)` (`EPHYRA_FAILURES`,
  the `failures` field of a JSON config, `#[ephyra::test(failures = n)]`)
  keeps searching past a failure until `n` distinct signatures are found or
  the budget ends. Signatures come from `ephyra::signature(name)` inside the
  property, else `Config::signature(f)` (given a `FailureInfo`), else the
  panic location, `returned false`, or the error message with numbers and
  quoted text blanked. Each new signature is shrunk while keeping it,
  remembered as a regression and listed in `Report::failures`
  (`DistinctFailure`: signature, count, failure); `Report::failure` stays
  the first. The text report adds a block per further failure. With the
  default of 1 nothing changes.
