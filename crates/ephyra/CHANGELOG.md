# Changelog

All notable changes to `ephyra` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

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
