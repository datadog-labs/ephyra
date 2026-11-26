# Changelog

All notable changes to `ephyra` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- `gens::f32()` could generate and mutate to `1.0`, outside its documented
  `[0, 1)` (#40). Fix by Yusuf Eren Bozkurt (#137).
- `gens::f64_range` returned `hi` on every draw when `hi - lo` overflows
  (`f64_range(f64::MIN..=f64::MAX)`), and its mutation step went to a bound
  (#41). Fix by Bruno Volpato (#69).
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
  settles, generation that spreads over the support), over generated arguments. It catches each of the bugs above.
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
