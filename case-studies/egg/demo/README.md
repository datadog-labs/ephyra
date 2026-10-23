# egg-demo

A showcase: equivalent SQL queries sampled from an e-graph, and checked on
SQLite. A seed (a schema and one `SELECT`) becomes a term; an
[egg](https://github.com/egraphs-good/egg) e-graph saturates it with sound
rewrite rules; terms of many sizes are sampled from the e-graph, the most
structurally different are kept, printed back as SQL, and each runs against
the seed on random SQLite databases (rusqlite, bundled SQLite), where it
must return the seed's rows as a multiset. A variant that disagrees or does
not run is a wrong rule or a printer bug. Seeds also come from an ephyra
generator, so such a bug shrinks to a small seed.

```
cargo run --release -- examples/left_join.sql [--n 8] [--samples 400] [--seed 1] [--out DIR]
cargo run --release -- check examples/*.sql --out DIR [--sqleq-bin DIR] [--qed PATH]
```

The first prints the seed, what the e-graph and the sampler did, the
chosen variants, and the verdict on 300 random databases; `--out` writes
each (seed, variant) pair as an input file for
[sqleq](https://github.com/datadog-labs/sqleq). `check` runs every pair, and
a one-operator mutant of each (a comparison or connective swapped, a LEFT
JOIN made inner, DISTINCT dropped, EXISTS or IN negated), past sqleq's
frontend, its provers ([QED](https://github.com/qed-solver/prover) and
`sqleq-solver`), its disprover (`sqleq-fuzz`) and the SQLite check, as one
cross-tab. Two cells are alarms and make it exit non-zero: a prover proving
a pair the fuzzer or SQLite refutes (unsound, or fed unfaithful IR), and a
pair this demo calls equivalent that either refutes. Tools come from a flag
or an environment variable, never a default: `--sqleq-bin DIR` or
`$SQLEQ_BIN` (a directory with `sqleq-frontend`, `sqleq-solver` and
`sqleq-fuzz`, such as a sqleq checkout's `target/release`), `--qed PATH` or
`$QED_PROVER`. A tool not given is reported as not run.

- [src/parse.rs](src/parse.rs): reads a script of `CREATE TABLE`s and one
  `SELECT` with sqlparser. The fragment: SELECT [DISTINCT] columns FROM
  tables joined by inner, cross and left joins, WHERE, a GROUP BY of exactly
  the projected columns (read as DISTINCT), and `IN (SELECT col ...)` and
  `EXISTS (SELECT ...)` subqueries. Anything else is refused by name.
- [src/lang.rs](src/lang.rs): the term language, e-class facts (provided
  and read aliases, NULL-strictness, which predicates reject a LEFT JOIN's
  padded rows, NOT NULL columns and single-column keys from the schema), and
  the rules. Every expression rule is an equality of SQL's three-valued
  logic, so it is sound anywhere, not only at the top of a WHERE: commutativity
  and associativity, De Morgan, negated comparisons, distribution, BETWEEN
  and IN-lists. Relational rules: inner-join reordering and filter
  push-down; LEFT JOIN filter movement, ON conjuncts on the null-supplied
  side, LEFT to inner under a padding-rejecting filter, LEFT JOIN
  associativity and commuting with a cross product; `IN` and `EXISTS` as
  whole filters, `IN` to a join under a key; DISTINCT as GROUP BY, and
  DISTINCT dropped when the projection outputs a key of every table.
- [src/sample.rs](src/sample.rs): `count` tabulates how many terms of each
  size every e-class holds (finite per size although the e-graph has
  cycles); `draw` picks a size uniformly, then a term of that size
  uniformly; `spread` keeps the `n` farthest by structural features
  (operator pairs, join order, size).
- [src/print.rs](src/print.rs): terms back to SQL.
- [src/sqlite.rs](src/sqlite.rs): random databases (NULLs unless NOT NULL,
  small numbers, the query's own literals) and the multiset comparison.
- [src/seed.rs](src/seed.rs): generated seeds inside the fragment: up to
  six tables of up to six columns (INTEGER, REAL, TEXT, foreign-key-style
  `t{j}_id`, some NOT NULL or UNIQUE), joins, filters, correlated and
  uncorrelated subqueries; valid by construction, with shrink candidates and
  mutants filtered through `well_formed`.
- [src/check.rs](src/check.rs): the `check` cross-tab.
- [tests/seeds.rs](tests/seeds.rs): `generated_seeds_parse` (the generator
  and the parser agree on the fragment) and
  `sampled_variants_agree_with_their_seed` (every sampled variant of a
  generated seed returns the seed's rows on random SQLite databases, and
  runs at all).
- [examples/](examples/): hand-written seeds (left joins, outer join chains,
  ranges, subqueries, DISTINCT, a three-way join).

Its PostgreSQL counterpart, over ephyra-postgresql's query language, is
[../../sqleq/checker](../../sqleq/checker).
