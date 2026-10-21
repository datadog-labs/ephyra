# ephyra-sqlite

A testing engine for SQLite on [ephyra](../../crates/ephyra) generators: a
generated database file and statements checked against an exact model, and
SQLancer-style metamorphic oracles comparing SQLite with itself on queries
over the state the statements leave. Every generator carries a validity
predicate and metrics, and its shrinker and mutator are verified by
`ephyra check`, so failures shrink to small, valid SQL.
[sqlite-coverage](../sqlite-coverage) scores the inputs by SQLite's own
line and branch coverage and runs the oracles unattended;
[diversity-probe](../diversity-probe) measures the inputs;
[ephyra-sqlite-pinned](../ephyra-sqlite-pinned) is an earlier copy that
[turso-diff](../turso-diff) uses. Bugs found:
[bugs/sqlite](../bugs/sqlite).

## How it works

Inputs are `(Case, Vec<Query>)`: a database file, statements to run on it,
and queries over the final schema
([probes.rs](src/probes.rs), `case_queries`, drawn with
`gens::dependent` so that shrinking and mutating the case repair the
queries). No generator hook runs SQLite.

- **Files** ([files.rs](src/files.rs), [model.rs](src/model.rs),
  [writer.rs](src/writer.rs), [reader.rs](src/reader.rs)): a logical
  database (`Model`: tables, WITHOUT ROWID and STRICT tables, indexes
  including partial and expression ones, views, triggers, foreign keys,
  generated columns, CHECK constraints, UTF-8 and UTF-16 encodings) and a
  physical `Layout` (page size, reserved bytes, fill, freelist, overflow
  chains, auto-vacuum), written byte by byte following the "Database File
  Format" document, not through SQLite. The reader parses those bytes back
  into the same pair, so shrinking and mutation work on the model and every
  candidate is valid by construction. The predicate is SQLite's own:
  `PRAGMA integrity_check` answers `ok` ([lib.rs](src/lib.rs),
  `integrity_ok`).
- **Statements** ([statements.rs](src/statements.rs),
  [semantics.rs](src/semantics.rs)): DML with conflict resolution, UPSERT
  and `UPDATE ... FROM`; CREATE/DROP TABLE and INDEX; ALTER TABLE (ADD,
  DROP, RENAME COLUMN, table swaps); views and triggers; foreign keys;
  savepoints and transactions; ATTACH and TEMP schemas; ANALYZE and crafted
  `sqlite_stat1` rows; VACUUM, `VACUUM INTO`, incremental vacuum; journal
  modes, WAL checkpoints and the storage pragmas. `semantics::apply` gives
  each statement's effect on the model and, for a SELECT, the rows SQLite
  must return. The language is restricted where the model could not
  predict SQLite exactly (affinity conversions it cannot tell, constraint
  check order, a random rowid); each restriction is checked by
  `semantics::valid` and documented in that module's docs.
- **Virtual tables** ([vtabs.rs](src/vtabs.rs)): R-Tree, Geopoly, FTS5
  and `fts5vocab` tables, with rows kept exactly as SQLite stores them.
- **Queries** ([expr.rs](src/expr.rs)): a general expression and query
  language checked against the schema, not against the model: joins of up
  to five sources (LEFT, RIGHT, FULL, CROSS, NATURAL, USING), subqueries,
  aggregates (with their own ORDER BY), window functions, compound SELECTs,
  CTEs (recursive and MATERIALIZED), JSON and date/time functions,
  collations, `INDEXED BY`. Results are kept determined by the data alone
  (total orders for LIMIT, no bare columns in aggregate queries), so any
  disagreement is SQLite's.

### Oracles

The model property ([tests/cases.rs](tests/cases.rs)): SQLite running a
case's statements on its file agrees with the model, statement by statement.

The metamorphic oracles ([sqlancer.rs](src/sqlancer.rs), each a property in
[tests/sqlancer.rs](tests/sqlancer.rs)):

| Oracle | Compares |
|---|---|
| `norec` | the optimized `count(*) ... WHERE p` against `sum((p) IS TRUE)` over every row (Rigger and Su, ESEC/FSE 2020) |
| `tlp` | a query against the union of its `p`, `NOT p`, `p IS NULL` partitions, for WHERE, DISTINCT, aggregates, GROUP BY and HAVING, as UNION ALL, INTERSECT and EXCEPT compounds (Rigger and Su, OOPSLA 2020) |
| `order` | ORDER BY and LIMIT against the unordered multiset |
| `setops` | a compound against its definition computed from its SELECTs' rows; commutative operands swapped |
| `ctes` | each CTE inlined as a subquery, and with MATERIALIZED and NOT MATERIALIZED |
| `windows` | window functions against their definition computed from the rows they read |
| `plans` | the query against plan variants a correct planner answers alike: `NOT INDEXED`, `INDEXED BY` each index, joins as CROSS JOIN, comma joins, reordered, LEFT/RIGHT mirrored, FULL as LEFT UNION ALL, USING/NATURAL as ON; and the query with all 31 optimizations off (`SQLITE_TESTCTRL_OPTIMIZATIONS`), a disagreement then named by the one optimization that reproduces it |
| `storage` | the database deserialized in memory against the same database in a WAL file through the unix VFS, with a small cache and checkpoints |

Each oracle's formulation, how results compare (multisets, sets, exact
reals) and its narrow exclusions (integer overflow and size limits that
depend on evaluation order, host I/O errors, JSON argument errors) are in
the module docs of [sqlancer.rs](src/sqlancer.rs).

## Build and run

The crate patches in SQLite trunk instead of the release bundled with
`libsqlite3-sys`, so that bugs found are ones trunk still has. Run
`case-studies/vendor/fetch.sh` first; it creates
`case-studies/vendor/libsqlite3-sys-trunk`.

```bash
cargo test --release                       # unit tests, the model property, every oracle
cargo test --release --test cases          # the model property
cargo test --release --test sqlancer       # the oracles, 1500 inputs each
ephyra check --no-instrument               # verify the generators: predicates, shrinkers, mutators, metrics
```

`ephyra` is the command of [crates/ephyra-cli](../../crates/ephyra-cli).

The examples run the same generator and oracle functions at scale:

```bash
# <norec|tlp|plans|order|setops|cte|window|storage> <first seed> <count>;
# a failure is shrunk and printed, and the database written to the dump dir
cargo run --release --example sqlancer -- tlp 0 100000 dump/ --queries 1-300 --mutate 2
cargo run --release --example sqlancer -- norec 0 10000 --wal DIR   # databases in WAL files in DIR (best a tmpfs)
cargo run --release --example heavy -- plans 0 2000      # inputs whose memory use is high
cargo run --release --example gap_probe                  # share of inputs reaching affinity/collation features
```

`--queries LO-HI` extends each input to a log-uniform number of queries and
`--mutate K` applies the generator's own mutation K times, as the
coverage-guided runner in [sqlite-coverage](../sqlite-coverage) draws its
inputs.

## What it found

Bugs in SQLite trunk, each with its report, repro and, where we wrote one, a
fix, under [bugs/sqlite](../bugs/sqlite) (index:
[bugs/README.md](../bugs/README.md)). Those reported on the SQLite bug forum:

| Bug | Report |
|---|---|
| [sqlite-savepoint-autovacuum](../bugs/sqlite/sqlite-savepoint-autovacuum/) | [3bc3c8adfa](https://sqlite.org/bugs/forumpost/3bc3c8adfa) |
| [sqlite-overflow-blob-encoding](../bugs/sqlite/sqlite-overflow-blob-encoding/) | [48c64d8bab](https://sqlite.org/bugs/forumpost/48c64d8bab) |
| [sqlite-right-join-using-star-ambiguous](../bugs/sqlite/sqlite-right-join-using-star-ambiguous/) | [8be1215e2c](https://sqlite.org/bugs/forumpost/8be1215e2c) |
| [sqlite-intersect-full-join-cte](../bugs/sqlite/sqlite-intersect-full-join-cte/) | [50dd78a44b](https://sqlite.org/bugs/forumpost/50dd78a44b) |
| [sqlite-groupby-subquery-orderby-direction](../bugs/sqlite/sqlite-groupby-subquery-orderby-direction/) | [5cd4b3b27a](https://sqlite.org/bugs/forumpost/5cd4b3b27a) |
| [sqlite-json-valid-stale-cache-json5](../bugs/sqlite/sqlite-json-valid-stale-cache-json5/) | [1339f93b28](https://sqlite.org/bugs/forumpost/1339f93b28) |
| [sqlite-expr-index-constant-propagation](../bugs/sqlite/sqlite-expr-index-constant-propagation/) | [529732a774](https://sqlite.org/bugs/forumpost/529732a774) |
| [sqlite-or-partial-index-covering-error](../bugs/sqlite/sqlite-or-partial-index-covering-error/) | [3ede79ce42](https://sqlite.org/bugs/forumpost/3ede79ce42) |
| [sqlite-partial-index-constant-affinity](../bugs/sqlite/sqlite-partial-index-constant-affinity/) | [35992eec46](https://sqlite.org/bugs/forumpost/35992eec46) |
| [sqlite-bloom-filter-rowid-real-minint](../bugs/sqlite/sqlite-bloom-filter-rowid-real-minint/) | [403ca95409](https://sqlite.org/bugs/forumpost/403ca95409) |
| [sqlite-aggregate-orderby-replaces-argument](../bugs/sqlite/sqlite-aggregate-orderby-replaces-argument/) | [b53a65d1fc](https://sqlite.org/bugs/forumpost/b53a65d1fc) |
| [sqlite-overflow-text-bom](../bugs/sqlite/sqlite-overflow-text-bom/) | [36a73e8c8a](https://sqlite.org/bugs/forumpost/36a73e8c8a) |
| [sqlite-in-select-collate-index](../bugs/sqlite/sqlite-in-select-collate-index/) | [f505c400cf](https://sqlite.org/bugs/forumpost/f505c400cf) |
| [sqlite-alter-default-blob-encoding](../bugs/sqlite/sqlite-alter-default-blob-encoding/) | [a9c654a346](https://sqlite.org/bugs/forumpost/a9c654a346) |
| [sqlite-gencol-index-real-int](../bugs/sqlite/sqlite-gencol-index-real-int/) | [6973ad169e](https://sqlite.org/bugs/forumpost/6973ad169e) |
| [sqlite-wr-bignull-added-column](../bugs/sqlite/sqlite-wr-bignull-added-column/) | [eee345f931](https://sqlite.org/bugs/forumpost/eee345f931) |
| [sqlite-concat-blob-utf16-length](../bugs/sqlite/sqlite-concat-blob-utf16-length/) | reported by someone else first: [c621e963a9](https://sqlite.org/bugs/forumpost/c621e963a9) |

[sqlite-left-join-in-null](../bugs/sqlite/sqlite-left-join-in-null/),
[sqlite-flatten-compound-right-join](../bugs/sqlite/sqlite-flatten-compound-right-join/)
and
[sqlite-window-left-join-outer-on](../bugs/sqlite/sqlite-window-left-join-outer-on/)
were reported on the bug forum too and fixed the same day (check-ins
e71b0465b9, cfa69ea48a, c50d7f682c).
[sqlite-group-term-in-select-affinity](../bugs/sqlite/sqlite-group-term-in-select-affinity/)
was already fixed on trunk when found. The other folders under
[bugs/sqlite](../bugs/sqlite), FTS5, R-Tree and Geopoly bugs among them,
hold reports posted or being posted after these; each folder has the text.
[sqlite-glob-index-blob](../bugs/sqlite/sqlite-glob-index-blob/) is not a
bug (invalid UTF-8 in, garbage out); the oracles skip it.
