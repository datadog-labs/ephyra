# Changelog

All notable changes to `ephyra` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.2.0] - 2026-10-30

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
