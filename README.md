# ephyra

Property-based testing for Rust, designed for tests written by coding
agents: one generator gives generation, shrinking, targeted search and exact
replay; every report is readable by people and by tools; every setting can
be changed from the environment.

See [`crates/ephyra/README.md`](crates/ephyra/README.md) for the
library's own README and the [changelog](crates/ephyra/CHANGELOG.md).

## Workspace

| Crate | Purpose | Status |
|---|---|---|
| [`crates/ephyra`](crates/ephyra) | The library: generators with verified shrinkers and mutators, targeted and coverage-guided search, seed replay, regressions, JSON reports, in-process coverage and fault localization | 0.2 (API not yet frozen) |
| [`crates/ephyra-cli`](crates/ephyra-cli) | The `ephyra` command: runs suites into one artifact directory, checks generators and strategies found in the source, and serves a local page for exploring and rerunning them | 0.2 |
| [`crates/ephyra-macros`](crates/ephyra-macros) | `#[derive(Arbitrary)]`, `#[ephyra::shrinker]`, `#[ephyra::test]` and `#[ephyra::generator]`, re-exported by `ephyra` | 0.2 |
| [`examples/locate-demo`](examples/locate-demo) | Standalone example of fault localization on the 0.2 core | Builds against `ephyra` with `-C instrument-coverage` |
| [`crates/ephyra-datadog`](crates/ephyra-datadog) | Sends test cases to Datadog as metrics over every run, with a dashboard | Unreleased |
| [`case-studies/`](case-studies) | ephyra applied to real systems; see below | Research |

## Case studies

[`case-studies/`](case-studies) holds projects that test real systems with
ephyra. They are not workspace members: each one builds on its own, against
the library here through `path = "../../crates/ephyra"`, and has a README
with what it tests, how to run it and what it found.

| Directory | What it tests |
|---|---|
| [`ephyra-sqlite`](case-studies/ephyra-sqlite), [`sqlite-coverage`](case-studies/sqlite-coverage), [`diversity-probe`](case-studies/diversity-probe) | SQLite: a typed SQL generator, a model and metamorphic oracles (TLP, NoREC, ...), and a coverage-guided runner over SQLite's C source |
| [`ephyra-sql`](case-studies/ephyra-sql), [`turso-diff`](case-studies/turso-diff), [`ephyra-sqlite-pinned`](case-studies/ephyra-sqlite-pinned) | Turso against SQLite (differential); `-pinned` is the SQLite generator as `turso-diff` uses it |
| [`ephyra-postgresql`](case-studies/ephyra-postgresql), [`postgresql-coverage`](case-studies/postgresql-coverage) | PostgreSQL: a generator, a model and oracles, and a coverage-guided runner |
| [`ephyra-sqleq`](case-studies/ephyra-sqleq) | [sqleq](https://github.com/datadog-labs/sqleq), on equivalent PostgreSQL queries from an e-graph over `ephyra-postgresql`'s seeds |
| [`ephyra-ada`](case-studies/ephyra-ada), [`ada-coverage`](case-studies/ada-coverage), [`ada-asan`](case-studies/ada-asan) | [ada](https://github.com/ada-url/ada), the WHATWG URL parser |
| [`ephyra-cedar`](case-studies/ephyra-cedar) | [Cedar](https://github.com/cedar-policy/cedar) policies and entities |
| [`egg-demo`](case-studies/egg-demo) | equivalent SQL queries sampled from an e-graph, checked on SQLite |
| [`bugs`](case-studies/bugs) | the bugs these found, by project, with the reports as filed, repros and fixes |

Some case studies build against code that is not in this repository:
`case-studies/vendor/fetch.sh` creates the `libsqlite3-sys` crates with
SQLite trunk that the SQLite ones patch in, and `turso-diff` and
`ephyra-cedar` need a checkout of Turso or Cedar at the repository root
(their READMEs give the revision).

## Developing

```bash
cargo test --workspace                                   # stable toolchain, MSRV 1.88
cargo clippy --workspace --all-targets -- -D warnings
cargo +nightly-2026-03-26 fmt --all                      # the pinned nightly CI uses; rustfmt.toml needs nightly options
cargo run -p ephyra --example bst -- --bug            # see a failure report
cargo run -p ephyra --example sort_performance        # see the targeted phase
cargo test -p ephyra --test reports                    # report snapshots; review drift with `cargo insta review`
RUSTFLAGS="-C instrument-coverage" RUST_TEST_THREADS=1 cargo test -p ephyra --features coverage   # instrumented mode
ci/package-smoke.sh                                       # build and test a consumer against the packaged crates
cargo run -p ephyra-cli -- test --features coverage   # artifacts into .ephyra/; see crates/ephyra-cli
```

The report text and JSON are snapshot-tested under `crates/ephyra/tests/reports/`;
a change to what a report says fails CI until the new snapshot is accepted.

## License

MIT. See [LICENSE](LICENSE).
