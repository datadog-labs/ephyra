# ephyra-postgresql

A testing engine for PostgreSQL, the port of [ephyra-sqlite](../../sqlite/generator):
a generated database and statements checked against an exact model, and
SQLancer-style oracles comparing PostgreSQL with itself, all on
[ephyra](../../../crates/ephyra) generators whose shrinkers and mutators
`ephyra check` verifies. [postgresql-coverage](../coverage) scores
its inputs by server code coverage and runs them unattended;
[ephyra-sqleq](../../sqleq/checker) draws its seeds from here. Bugs found:
[bugs/postgresql](../bugs).

## Setup

- **PostgreSQL**: a PostgreSQL master built with `--enable-cassert` and
  installed with contrib (amcheck, btree_gin, btree_gist), binaries in
  `EPHYRA_PG_BIN` (default `~/Projects/postgres/inst/bin`). The build used
  here: `--enable-cassert --enable-debug --with-icu --with-lz4 --with-zstd
  --without-readline CFLAGS="-O2 -g"`, `make world-bin install-world-bin`
  (`--with-lz4` is needed: the generator sets lz4 column compression).
  Assertions are on: a crash is a finding.
  [postgresql-coverage/deploy/build-pg.sh](../coverage/deploy/build-pg.sh)
  builds one.
- **The cluster** ([src/pg.rs](src/pg.rs)): `.pg/data`, socket in `.pg`, log
  `.pg/server.log` (`EPHYRA_PG_DIR` overrides the directory); started on first
  use by any test (initdb with `--locale=C -E UTF8`,
  fsync/full_page_writes/synchronous_commit off, autovacuum off) and left
  running. `pg_ctl -D .pg/data stop` stops it. psql: `$EPHYRA_PG_BIN/psql -h
  $PWD/.pg -U postgres`.
- Each thread's session has a **database of its own** (`e{pid}_{n}`, schema
  `s`), recreated per case; databases of dead processes are dropped on the
  next connect. Never share a database between threads: a database-wide
  VACUUM/ANALYZE also processes the shared catalogs, and REINDEX
  CONCURRENTLY waits for older snapshots, so threads deadlocked or timed
  out. For the same reason VACUUM and ANALYZE always name their tables.
- `ephyra check --no-instrument` here verifies the generators (the `ephyra`
  command is [crates/ephyra-cli](../../../crates/ephyra-cli)).

```bash
cargo test --release --test cases          # the model property
cargo test --release --test sqlancer       # every oracle
cargo run --release --example sqlancer -- all 0 1000 8
cargo run --release --example dump -- 42   # an input as a psql script
```

## What exists (and how it maps to ephyra-sqlite)

- [model.rs](src/model.rs) (sqlite's `model.rs`): typed values (int2/4/8,
  float4/8, numeric with scale, text in the C collation, bool, bytea, date,
  timestamp) compared as PostgreSQL compares them (NaN largest, `-0 = 0`,
  `1.0 = 1.00`, ASC NULLS LAST, dates past the last timestamp); tables,
  columns and indexes named by stable ids (`t{id}`, `c{id}`, `t{t}_i{id}`),
  so they can be dropped in any order. Identity sequences live outside the
  transactional state (`Model::sequences`). `Table::widest` tracks the
  widest value each column has held: CREATE INDEX indexes recently dead row
  versions too, so a new index must fit old values.
- [statements.rs](src/statements.rs), [semantics.rs](src/semantics.rs): DML,
  TRUNCATE, CREATE/DROP TABLE and INDEX (btree/hash/brin, partial,
  expression, INCLUDE, NULLS NOT DISTINCT, storage params), ALTER TABLE ADD
  COLUMN (fast default)/DROP COLUMN/SET NOT NULL/SET DEFAULT, VACUUM [FULL,
  FREEZE, ANALYZE], ANALYZE, CLUSTER, REINDEX [CONCURRENTLY], CHECKPOINT,
  planner SETs, crafted statistics
  (`pg_restore_relation_stats`/`pg_restore_attribute_stats`; at most 1000
  tuples per page, since a hash index build sizes its buckets from the
  planner's estimate), savepoints (`BEGIN; SAVEPOINT s0` is savepoint 0,
  `RELEASE s0` is `COMMIT`), storage and estimation changes (`Phys`:
  fillfactor, SET LOGGED/UNLOGGED, per-column statistics targets and
  n_distinct, CREATE/DROP STATISTICS, CLUSTER ON), and writes to referenced
  rows. Restrictions documented in `semantics.rs`'s module docs.
- [fk.rs](src/fk.rs): foreign keys (single- and multi-column; NO ACTION,
  RESTRICT, CASCADE, SET NULL, SET DEFAULT with column lists; MATCH
  SIMPLE/FULL; DEFERRABLE and SET CONSTRAINTS; NOT VALID and VALIDATE;
  self-references; to and from partitioned tables) with an exact model of
  their actions (breadth first over a forest of keys) and errors (23503,
  23001, and what an action's own rows raise; a statement that must fail runs
  in a DO block catching only that code); and PL/pgSQL triggers (BEFORE ROW
  set or skip with WHEN and UPDATE OF, AFTER ROW and transition-table audit
  writes, ENABLE/DISABLE), run in PostgreSQL's order by the model. Not
  modelled: deferred violations fixed later or failing at COMMIT, NOT
  ENFORCED keys, deferrable constraint triggers, triggers on partitioned
  tables.
- [setup.rs](src/setup.rs) (sqlite's `files.rs` + writer/reader): `Setup` =
  model + layout; the layout is what shapes pages without changing rows
  (fillfactor, toast_tuple_target, UNLOGGED, COPY vs INSERT batches, indexes
  before or after the rows, ghost rows loaded and deleted, a `SET c = c`
  touch, VACUUM/FREEZE/FULL/CLUSTER after loading, ANALYZE). `normalize`
  canonicalizes choices the script would not show (else ephyra check sees a
  shrink candidate render as its input); `Setup`'s Debug carries a digest of
  the whole script.
- [cases.rs](src/cases.rs), [tests/cases.rs](tests/cases.rs): the
  model-agreement property (query rows, rows-written counts, final contents,
  `verify_heapam`, `bt_index_check`).
- [expr.rs](src/expr.rs), [probes.rs](src/probes.rs),
  [sqlancer.rs](src/sqlancer.rs), [tests/sqlancer.rs](tests/sqlancer.rs): the
  typed query language (drawn and checked by type, literals always cast, a
  node budget per query; jsonb, jsonpath, arrays and interval arithmetic as
  expression-only types) and the oracles NoREC, TLP, plans (12
  planner-setting variants run under `BEGIN; SET LOCAL ..; ROLLBACK`), order,
  setops, cte, window, generic, and views (`CREATE VIEW` of the query,
  `SELECT *` from it, both texts of `pg_get_viewdef` run as queries, the
  deparsed text's fixed point, `EXPLAIN (VERBOSE)` in all four formats,
  `EXPLAIN ANALYZE`'s top-node row count). Queries include grouping sets
  (ROLLUP, CUBE, GROUPING SETS), and setups extended statistics (CREATE
  STATISTICS before the ANALYZE). Results compare as values PostgreSQL
  compares equal (`sqlancer::Canon`); plan-dependent run-time errors
  (overflow, out of range) are a counted skip. Known PostgreSQL bugs the
  oracles skip are listed in `sqlancer.rs`'s module docs and
  `sqlancer::known_error`.
- [examples/sqlancer.rs](examples/sqlancer.rs): `cargo run --release
  --example sqlancer -- <norec|tlp|plans|order|setops|cte|generic|window|views|all>
  <first seed> <count> [threads]`; [examples/dump.rs](examples/dump.rs)
  `<seed> [size]` prints an input as a psql script.

Generators `setups`, `cases`, `case_queries` pass `ephyra check` (10000
each). A burn of 6000 seeds per oracle (8 threads, about 290000
comparisons) found only false positives of a numeric zero's scale (`0.000`
vs `0`, fixed in `Canon`).

## Not ported yet (gaps against ephyra-sqlite)

- LIKE escapes and collations other than C.
- A storage oracle (sqlite's memory vs WAL): candidates are a cluster
  restart or crash recovery between statements, or `pg_dump`/restore.
- Inheritance, views as generated objects (the views oracle makes its own),
  ALTER COLUMN TYPE, MERGE, INSERT .. ON CONFLICT, RETURNING, set-returning
  functions in FROM, LATERAL, range types, full-text search (measured gaps:
  [postgresql-coverage/gaps/2026-10-16.md](../coverage/gaps/2026-10-16.md)).
