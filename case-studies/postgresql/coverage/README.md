# postgresql-coverage

PostgreSQL code coverage of [ephyra-postgresql](../generator)'s
inputs: the analogue of [sqlite-coverage](../../sqlite/coverage)'s qscore, for
scoring generator changes. Also the PostgreSQL fuzz runner (guide in
[RUNNER.md](RUNNER.md)) and its deployment and pre-triage
([deploy/README.md](deploy/README.md), [deploy/PRETRIAGE.md](deploy/PRETRIAGE.md)).

- **Instrumented PostgreSQL**: master built with clang's source-based
  coverage: `CFLAGS="-O1 -g -fprofile-instr-generate -fcoverage-mapping"
  LDFLAGS=-fprofile-instr-generate --enable-cassert --enable-debug
  --without-icu --with-lz4 --with-zstd --without-readline`, installed with
  contrib, at `~/Projects/postgres-cov/inst/bin` (`src/lib.rs`'s `bin_dir`,
  fixed: no environment override), from a checkout in a directory named
  `postgres-cov` (`functions.tsv` keeps source paths after `/postgres-cov/`).
  `deploy/build-pg.sh --coverage` builds one in `PG_BUILDS/<commit12>-cov/`;
  link or move it there. libpq's "no exit()" check fails on the profile
  runtime's `atexit`; `touch src/interfaces/libpq/libpq-refs-stamp` skips it
  (PostgreSQL's own `--enable-coverage` skips it too).
- **The cluster**: `.pg` here, started by the first session with
  `LLVM_PROFILE_FILE=.pg/prof/%m_%p.profraw`. Backends are forked, not
  exec'd, so all of them merge into the postmaster's file
  (`<sig>_0_<postmaster pid>.profraw`) at exit. Restarting the cluster starts
  a new file. Long-lived processes flush only at shutdown (not counted).
- [src/lib.rs](src/lib.rs): raw profile parsing (version 10), snapshots of
  the merged file under a shared lock, the counter-to-file map
  `functions.tsv` (function name, source file).
- `functions.tsv` is generated and not in the repository (13 MB, specific to
  the instrumented build): `cargo run --release --bin functions` starts the
  instrumented cluster, runs one session so the profile exists, merges it
  with `xcrun llvm-profdata` and writes each function's name and file from
  `xcrun llvm-cov export` of the instrumented `postgres` (macOS as written:
  on Linux, replace the `xcrun` calls in `write_function_files` with
  `llvm-profdata`/`llvm-cov` of the clang that built PostgreSQL).
  Regenerate it after rebuilding the instrumented PostgreSQL; qscore reads
  it.
- `qscore`: `cargo run --release --bin qscore -- [--seed S] [--inputs N]
  [--dirs OUT.tsv]`: batches of 10 inputs, one session each; score `Σ_c`
  (share of batches whose backend grew counter c), counters in
  `src/backend`. First measures (2026-10-13, master 158b6d5109): seed 0, 200
  inputs: 29062.5 and 29069.9 (two runs), seed 1: 29040.5; about 5-8
  inputs/s. Measured gaps of the generator:
  [gaps/2026-10-16.md](gaps/2026-10-16.md).
- `runner` (`src/bin/runner/`, guide in [RUNNER.md](RUNNER.md)): the fuzz
  runner, sqlite-coverage's runner ported: a supervisor, workers on one
  assertion-enabled cluster of the run's own (`D/pg`), the model property
  and nine oracles in turn, crash/hang/memory handling, records and
  `ephyra.postgresql.*` metrics over DogStatsD (optional). Not
  coverage-guided. It is not part of the frozen scorer (its own modules and
  `build.rs`, which only sets `RUNNER_BUILT_VERSION`).
- `deploy/`: scripts that build PostgreSQL master, run the runner
  unattended on a Linux host with a watchdog and a nightly master refresh,
  and pre-triage findings against the known bugs of
  [deploy/known-bugs.json](deploy/known-bugs.json) (paths relative to
  `case-studies/`; the bugs' folders are under [../bugs](../bugs)).
- Not yet: held-out monitors, a control (what any input reaches, e.g.
  connection setup, is in every batch), frozen-scorer checksums.
