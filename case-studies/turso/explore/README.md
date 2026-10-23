# ephyra-sql

A SQL testing engine for [Turso](https://github.com/tursodatabase/turso),
built as an [ephyra](../../../crates/ephyra) loop: SQL cases from a typed AST
whose generators, shrinkers and mutators `ephyra check` verifies, run
against Turso and SQLite under differential and metamorphic oracles, with
coverage-guided search inside `turso_core`. The first of the Turso case
studies, and small; [turso-diff](../differential) took over the large-scale
differential testing with [ephyra-sqlite](../../sqlite/generator)'s much richer
generator.

What it adds over Turso's own differential fuzzers is what ephyra provides:
structure-aware shrinking to minimal SQL scripts that is checked to keep
cases valid, coverage feedback from inside `turso_core`, causality analysis
of a failing script, and generator quality made visible as metrics.

## How it works

- **AST** ([ast.rs](src/ast.rs)): one table `t` with columns `c0, c1, ...`
  of INTEGER, TEXT or REAL, rows of literals (NULL included), and one
  `SELECT <columns | *> FROM t [WHERE e] [ORDER BY ...] [LIMIT n]`;
  expressions are columns, literals, comparisons, `AND`/`OR`/`NOT`,
  `IS NULL`, arithmetic and `LIKE`. `Display` renders a case as a runnable
  SQL script, so a counterexample prints as one.
- **Generator** ([generate.rs](src/generate.rs)): `cases()` with the
  validity predicate `well_formed` (rows fit the schema, references name
  columns, REALs finite, filter depth bounded, `LIMIT` only with
  `ORDER BY`) and `case_metrics` (rows, columns, filter depth and nodes,
  NULL share, columns the filter reads). Shrinking drops `LIMIT`, the
  filter or a projected column, replaces a subexpression by a child, and
  moves literals toward zero or the empty string.
- **Scripts** ([script.rs](src/script.rs)): stateful cases, a table and up
  to 200 steps (INSERT, `INSERT INTO t SELECT * FROM t`, UPDATE, DELETE,
  CREATE INDEX, SELECT), generated against a live SQLite so that later
  steps use the rows and values earlier ones left. Scripts implement
  ephyra's `Trace`: generation can resume after any prefix, which causality
  analysis uses.
- **Engines** ([engine.rs](src/engine.rs)): Turso through `turso_core`,
  synchronously, in memory; SQLite through rusqlite. Outcomes are rows or
  an error; error messages are kept for the report but not compared
  (Turso's error kinds are coarser than SQLite's). Rows compare in order
  under `ORDER BY`, else as multisets, REALs exactly.
- **Observations** ([observe.rs](src/observe.rs)): features, labels and
  coverage requirements recorded from inside a property, for the report
  and for observation files (`EPHYRA_OBSERVATIONS`).
- **Search** ([lib.rs](src/lib.rs)): `explore`, a loop on ephyra's `Run`
  that alternates fresh cases with mutants of cases that reached new
  `turso_core` coverage, weighted by how much they reached;
  `explore_and_explain` adds causality analysis (rewinding a failing
  script to earlier steps and continuing from each many times) before
  shrinking. Coverage is filtered to `turso_core`, so novelty and fault
  localization are about the engine.

Properties:

| Property | File | Checks |
|---|---|---|
| `turso_agrees_with_sqlite` (and `_explored`, under the coverage loop) | [tests/properties.rs](tests/properties.rs) | the same rows, or both an error |
| `where_partitions_rows` | [tests/properties.rs](tests/properties.rs) | TLP in Turso alone: `WHERE p`, `WHERE NOT p` and `WHERE p IS NULL` partition the unfiltered rows |
| `scripts_agree` | [tests/scripts.rs](tests/scripts.rs) | after every step, the same rows or both an error, the same number of rows changed, the same table |

The reference is rusqlite 0.37's bundled SQLite 3.50.2, next to the 3.50.4
Turso tracks; a much later SQLite would show its own changes as false
differences (3.53 parses some REALs one ulp apart). Turso is `turso_core`
0.8.1 from crates.io.

## Build and run

```bash
cargo test                            # plain build: random testing
ephyra check -- --features coverage   # verify the generators, on an instrumented build
```

`ephyra` is the command of [crates/ephyra-cli](../../../crates/ephyra-cli).
Coverage-guided runs need an instrumented build of `turso_core`; a separate
target directory keeps it apart from the plain build, and
[.cargo/config.toml](.cargo/config.toml) puts profile files under
`target/profraw/`:

```bash
RUSTFLAGS="-C instrument-coverage" RUST_TEST_THREADS=1 \
  cargo test --features coverage --target-dir target/cov
```

## What it found

[bugs/turso-affinity-trailing-zeros.md](bugs/turso-affinity-trailing-zeros.md),
found by `scripts_agree` under coverage guidance: text such as `'1.0e-300'`
stored in a REAL column becomes a double one ulp away from SQLite's and
from Turso's own `CAST`. Reported as
[tursodatabase/turso#9849](https://github.com/tursodatabase/turso/issues/9849)
together with the wider form turso-diff later found
([case-studies/turso/bugs/turso-real-affinity-parse](../bugs/turso-real-affinity-parse/)).
