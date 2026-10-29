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

## Using

```toml
[dev-dependencies]
ephyra = "0.2"
```

Write a property as a test:

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

`cargo test` runs it. A failure prints the shrunk counterexample and an
`EPHYRA_REPLAY=<seed>:<iteration>` line that reruns exactly that case, and is
remembered in `.ephyra/regressions/` so the next run tries it first. Settings
such as the number of tests (`EPHYRA_TESTS`), the seed (`EPHYRA_SEED`) or a
time budget (`EPHYRA_BUDGET_MS`) can be changed from the environment without
touching the code.

The `ephyra` command runs suites into one artifact directory, checks the
generators it finds in the source, and serves a local page for exploring the
results:

```bash
cargo install ephyra-cli
ephyra guide          # how to write generators
ephyra test           # run the suite; artifacts in .ephyra/
```

To see a failure report and the targeted search from a checkout:

```bash
cargo run -p ephyra --example bst -- --bug
cargo run -p ephyra --example sort_performance
```

Next: [the library's README](crates/ephyra/README.md) for generators,
shrinkers and mutators, [API.md](API.md) for every generator, combinator and
setting, and [CONTRIBUTING.md](CONTRIBUTING.md) to work on ephyra itself.

## Case studies

![Bugs the case studies found and reported, one row per project: each dot is a bug, placed at how long it had been in the code when it was found, over a bar for the project's history.](case-studies/bug-ages.svg)

Each dot is a bug found with ephyra and reported upstream, placed at how long
it had been in the project's code when it was found; the bar behind each row is
the project's history since its first commit.

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
| [`coreutils`](case-studies/coreutils), [`duckdb`](case-studies/duckdb), [`datafusion`](case-studies/datafusion), [`brush`](case-studies/brush), [`gix`](case-studies/gix), [`binaryen`](case-studies/binaryen), [`ocaml`](case-studies/ocaml), [`oxcaml`](case-studies/oxcaml), [`rustc`](case-studies/rustc), [`litellm`](case-studies/litellm) | `harness` | targets of ephyra's autonomous agent: the harness it wrote for each (uutils coreutils, DuckDB, DataFusion, brush, gitoxide, Binaryen, OCaml, OxCaml, rustc, LiteLLM) and the bugs it found |

Some case studies build against code that is not in this repository:
`case-studies/sqlite/vendor/fetch.sh` creates the `libsqlite3-sys` crates with
SQLite trunk that the SQLite and Turso ones patch in, and `turso/differential`
and `cedar/generator` need a checkout of Turso or Cedar at the repository root
(their READMEs give the revision).

## License

MIT. See [LICENSE](LICENSE).
