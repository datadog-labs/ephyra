# ephyra

Property-based testing for Rust, designed for tests written by coding
agents: one generator gives generation, shrinking, targeted search and exact
replay; every report is readable by people and by tools; every setting can
be changed from the environment.

See [`crates/ephyra/README.md`](crates/ephyra/README.md) for the
library's own README and the [changelog](crates/ephyra/CHANGELOG.md).

**Testing a project with an agent?** Point it at [STARTUP.md](STARTUP.md):
"Let's test `<my project>` with ephyra; start from
https://github.com/datadog-labs/ephyra/blob/main/STARTUP.md".

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
ephyra, one directory per target system: a crate per component, and `bugs/`
with what they found (the reports as filed upstream, repros and fixes, and a
`README.md` index with each bug's status). The crates are not workspace
members: each one builds on its own, against the library here through
`path = "../../../crates/ephyra"`, and has a README with what it tests, how to
run it and what it found.

| Target | Components | What they test |
|---|---|---|
| [`sqlite`](case-studies/sqlite) | [`generator`](case-studies/sqlite/generator), [`coverage`](case-studies/sqlite/coverage), [`diversity`](case-studies/sqlite/diversity) | SQLite: a typed SQL generator, a model and metamorphic oracles (TLP, NoREC, ...), and a coverage-guided runner over SQLite's C source |
| [`turso`](case-studies/turso) | [`explore`](case-studies/turso/explore), [`differential`](case-studies/turso/differential) | Turso against SQLite (differential), on the SQLite generator's inputs |
| [`postgresql`](case-studies/postgresql) | [`generator`](case-studies/postgresql/generator), [`coverage`](case-studies/postgresql/coverage) | PostgreSQL: a generator, a model and oracles, and a coverage-guided runner |
| [`sqleq`](case-studies/sqleq) | [`checker`](case-studies/sqleq/checker) | [sqleq](https://github.com/datadog-labs/sqleq), on equivalent PostgreSQL queries from an e-graph over `postgresql/generator`'s seeds |
| [`ada`](case-studies/ada) | [`generator`](case-studies/ada/generator), [`coverage`](case-studies/ada/coverage), [`asan`](case-studies/ada/asan) | [ada](https://github.com/ada-url/ada), the WHATWG URL parser |
| [`cedar`](case-studies/cedar) | [`generator`](case-studies/cedar/generator) | [Cedar](https://github.com/cedar-policy/cedar) policies and entities |
| [`egg`](case-studies/egg) | [`demo`](case-studies/egg/demo) | equivalent SQL queries sampled from an e-graph, checked on SQLite |
| [`sqlparser`](case-studies/sqlparser) | | [sqlparser-rs](https://github.com/apache/datafusion-sqlparser-rs): bugs from round-trip and statement-separation properties an agent wrote from STARTUP.md alone (harness to follow) |
| [`coreutils`](case-studies/coreutils), [`duckdb`](case-studies/duckdb), [`datafusion`](case-studies/datafusion), [`brush`](case-studies/brush), [`gix`](case-studies/gix), [`binaryen`](case-studies/binaryen), [`ocaml`](case-studies/ocaml), [`oxcaml`](case-studies/oxcaml), [`rustc`](case-studies/rustc) | `harness` | targets of ephyra's autonomous agent: the harness it wrote for each (uutils coreutils, DuckDB, DataFusion, brush, gitoxide, Binaryen, OCaml, OxCaml, rustc) and the bugs it found |

Some case studies build against code that is not in this repository:
`case-studies/sqlite/vendor/fetch.sh` creates the `libsqlite3-sys` crates with
SQLite trunk that the SQLite and Turso ones patch in, and `turso/differential`
and `cedar/generator` need a checkout of Turso or Cedar at the repository root
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
