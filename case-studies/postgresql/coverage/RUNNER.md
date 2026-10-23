# The PostgreSQL fuzz runner

`src/bin/runner/` is the port of `../../sqlite/coverage`'s fuzz runner to
PostgreSQL: a supervisor, worker processes, and one PostgreSQL cluster
they share, for unattended runs of hours or days. Records, layout and
metric names follow the SQLite runner's (`../../sqlite/coverage/deploy/README.md`),
so its scripts carry over with `sqlite` replaced by
`postgresql`. It is separate from `qscore` and the library (the frozen
scorer): it adds a build script (`build.rs`, the runner's version) and a
binary, and changes neither.

```bash
cargo build --release --bin runner
target/release/runner run --dir ~/fuzz-runs/pg-1 --workers 8 --hours 24 \
  --statsd 127.0.0.1:8125 --tags env:fuzz
```

## CLI

- `runner run --dir D [--workers N] [--hours H] [--seed S] [--campaign C]
  [--tag T] [--statsd HOST:PORT] [--tags k:v,..] [--max-rss-mb MB]
  [--stale-secs T] [--pg-bin DIR] [--pg-dir DIR]`: the supervisor. Defaults:
  4 workers, 24 hours, the seed is the start time, campaign
  `postgresql/campaign/runner`, tag `untagged`, no metrics, 4096 MB per
  process (worker or backend), a hang after 120 s without a heartbeat,
  binaries in `EPHYRA_PG_BIN` (default `~/Projects/postgres/inst/bin`, a
  PostgreSQL master built with `--enable-cassert`), the
  cluster in `D/pg`. The seed, the deadline (`D/until`) and the cluster
  directory (`D/pg_dir` when `--pg-dir` is given) are fixed at the first
  start; a restarted supervisor (same command) resumes the run.
- `runner worker --dir D --id K --seed S --until T [--statsd ..] [--tags ..]`:
  one worker, spawned by the supervisor (it inherits `EPHYRA_PG_DIR` and
  `EPHYRA_PG_BIN`).
- `runner replay --finding FILE|DIR --out DIR [--dir D | --pg-dir DIR]
  [--pg-bin DIR] [--oracle NAME]`: rebuilds an input from a finding (an
  example directory or its `state.txt`, a crash's `w<K>.state.txt`, a hang,
  memory or worker-crash record) from its seed and size, writes
  `script.sql` (psql), `input.txt` (Debug), `state.txt`, and runs its oracle
  once against the run's cluster (found from the finding's path, started if
  it is stopped) or the one given; `verdict.txt` has the outcome. It prints
  the psql command for the script. A failure's example is rebuilt unshrunk;
  its shrunk `script.sql` is beside it.
- `runner version`: the version of the sources on disk and the one the
  binary was built from; exits 1 when they differ.

## What a worker does

Input `n` of worker `K` is a fresh draw of ephyra-postgresql's
`case_queries` from seed `mix(mix(S, K), n)` at size `n % 101`, checked by
`ORACLES[n % 10]`: `model` (the model property of `tests/cases.rs`,
reimplemented as a function in `src/bin/runner/model.rs`; keep the two
equivalent), `norec`, `tlp`, `plans`, `order`, `setops`, `cte`, `window`,
`generic`, `views`. A worker process is one session: one database `e<pid>_0`
(ephyra-postgresql's `pg.rs`; databases of dead workers are dropped when
the next one connects). After 500 inputs (`worker::RENEW_EVERY`;
`EPHYRA_RUNNER_RENEW_EVERY` overrides it) a worker saves its totals, closes
its session and exits with `RENEW_EXIT`, and the supervisor starts it again
at once (an event `worker_renew`, no record, no restart counted): a backend
that lives for days gets slower. Outcomes:

- pass: the tally is added (queries, comparisons per variant, skips);
- failure: shrunk with the generator's shrinker (2 minutes,
  `EPHYRA_RUNNER_SHRINK_SECS`), recorded under its signature;
- known PostgreSQL bug (`sqlancer::known_error`) where the oracle does not
  already skip it: counted as `known`, not a finding (the oracles' own skips
  are `oracle.skips{reason:known_skips}`);
- statement timeout outside the oracles' skips: counted, the first 5 per
  worker process saved under `findings/timeouts/`;
- session lost (crash, restart, server refusing connections): the worker
  writes `workers/K/lost` (+ `lost.sql` when it was shrinking: the
  candidate that ran) and exits with code 3.

No coverage-guided loops in v1: every backend of a postmaster merges into
one profile file, so one worker's coverage cannot be told apart. The seam
is `worker::draw` (a pool would choose inputs there) and a cluster per
guided worker (`--pg-dir` and `LLVM_PROFILE_FILE` per worker).

## What the supervisor does

- Creates and starts the cluster with `pg.rs`'s settings (initdb `--locale=C
  -E UTF8`, fsync/full_page_writes/synchronous_commit off, autovacuum off,
  200 connections, shared_buffers 256MB), never the shared
  `ephyra-postgresql/.pg`. Stops it (`pg_ctl stop -m fast`) at the end. The
  socket path `D/pg/.s.PGSQL.5432` must stay under ~100 bytes: use a short
  `--dir` (or `--pg-dir`).
- Every second, reads what `server.log` gained. A crash line (`TRAP:`,
  `PANIC:`, a server process "terminated by signal", "exited with exit code"
  other than 0 or 1, "all server processes terminated") starts the crash
  path: wait up to 15 s for the workers to exit, record the crash with the
  log excerpt and every worker's input in flight, kill what is left, wait
  for crash recovery (`pg_isready`; start the cluster when the postmaster is
  gone; after 3 minutes `pg_ctl stop -m immediate` and start; after 10
  minutes give up and stop the run), restart all workers. A restart caused
  only by the supervisor's own kills is a `server_restart` event, not a
  crash. A worker that lost its session with no crash in the log is
  restarted after 20 s if the server answers.
- Memory: a thread reads `ps` every 500 ms and SIGKILLs any client backend
  (`[local]` in its title) of the run's postmaster over `--max-rss-mb`,
  recording the owning worker's input (`findings/memory/`). That makes the
  postmaster restart every session (recorded as `server_restart`). Worker
  processes over the limit are killed every 5 s check. RSS on macOS includes
  touched shared buffers (up to 256 MB).
- Hangs: a worker whose stats file is older than `--stale-secs` is killed,
  recorded (`findings/hangs/`), its backend sent SIGTERM and, 3 s later,
  SIGKILL (a parse-analysis loop never reads interrupts).
- Restarts workers that exit (signal: `findings/worker-crashes/`, other
  codes: `findings/errors/`), gives one up after 10 restarts in 10
  minutes. On start, kills workers a dead supervisor left (`workers/K/pid`).
- Every minute: `stats.tsv`, a `total` line in `snapshots.jsonl`, metrics.
- Stops at the deadline or when `D/stop` exists; workers get 90 s to finish
  their input, then are killed.

## Layout of `D`

| Path | What |
| --- | --- |
| `run.json` | header, schema `ephyra.run.v1`: run id, campaign, tag, seed, config, host, version (content hashes and commits of ephyra, ephyra-postgresql, postgresql-coverage; binary hash), target (`postgres -V`, the checkout's commit, `pg_config --configure`) |
| `events.jsonl`, `events.log` | events (below), JSON and readable |
| `snapshots.jsonl` | `worker` lines every 30 s, `total` lines every minute (by oracle too) |
| `stats.tsv` | one line a minute: time, elapsed (since this supervisor started), alive, restarts, crashes, hangs, memory_kills, server_restarts, execs, execs_per_sec, failures, timeouts, known, gen_panics, lost, signatures, crash_signatures, backends, backend_max_mb |
| `seed`, `until`, `pg_dir`, `supervisor.state`, `supervisor.pid` | what a restarted supervisor resumes from (counters, server-log offset) |
| `pg/` | the cluster: `data/`, socket, `server.log` |
| `workers/K/` | `state` (input in flight), `stats` (totals, heartbeat, backend pid), `lost`, `lost.sql`, `pid`, `log` |
| `findings/` | below |

Events (`kind`): `run_start`, `stale_binary`, `version_changed`,
`server_start`, `worker_spawn`, `worker_start`, `worker_stop`,
`worker_no_session`, `session_lost`, `finding`, `timeout_saved`,
`generator_panic`, `crash`, `server_restart`, `server_down`,
`server_start_failed`, `server_ready`, `server_unrecoverable`,
`memory_kill`, `hang`, `worker_crash`, `error`, `worker_lost`,
`worker_given_up`, `run_stopping`, `run_stop`.

## Findings and signatures

- `findings/<signature>/`: oracle and model failures. Signature = oracle,
  `-`, the message's first line with every digit run as `N` and other
  characters as `_` (`plans-plans-enable_hashjoin__results_differ`,
  `model-model__rows_differ`). A first line that holds SQL is replaced by
  its kind, the statement's first word and the SQLSTATE
  (`model-statement-UPDATE-XX000`, `norec-original-fails-SELECT-42883`).
  Shrinking keeps the signature. `finding.json` (first seen, `status:
  open`), `count`, and up to 20 examples `w<K>-<n>/`: `message.txt`,
  `input.txt` (shrunk Debug), `script.sql` (shrunk, psql: settings,
  extensions, setup with COPY data inline, statements, COMMIT if needed,
  queries), `state.txt` (the unshrunk draw), `example.json`.
- `findings/crashes/<signature>/<time>/`: server crashes. Signature from the
  log: `trap-<assertion with File and Line, PID cut>`, `panic-<message>`,
  `signal-<N>`, `exit-<code>`, or `server-lost`. `crash.txt` (crash lines,
  the "Failed process was running" query, the log's last 300 lines, every
  worker's input in flight; culprits are the workers whose backend pid the
  log names), `w<K>.state.txt` and `w<K>.sql` (each in-flight input rebuilt
  from its seed), `w<K>.candidate.sql` when a worker was shrinking.
- `findings/hangs/`, `memory/`, `worker-crashes/`, `errors/`,
  `generator-panics/`, `timeouts/`: one record per event, with the input in
  flight.

Reproducing: `psql -X -h D/pg -U postgres -d <a new database> -f
script.sql`, or `runner replay --finding <example dir> --out DIR`. Before
reporting anything, check PostgreSQL master and the mailing-list archives.

## Metrics

DogStatsD over UDP, only with `--statsd`. Every metric is tagged `run`,
`campaign`, `work_tag`, `version`, `target:postgresql-<commit12>` and
`--tags`. Same names as the SQLite runner's with `ephyra.postgresql.` for
`ephyra.sqlite.`; the extra PostgreSQL ones are marked *.

Counts per minute (`|c`): `execs`, `failures`, `timeouts`, `known_bugs`*,
`generator_panics`, `sessions_lost`*, `inputs{kind:fresh}`;
`oracle.execs`, `oracle.failures`, `oracle.timeouts`, `oracle.known`*,
`oracle.ms`, `oracle.queries`, `oracle.comparisons` (tag `oracle`);
`oracle.skips` (tags `oracle`, `reason`: `inapplicable`, `error_skips`,
`timeouts`, `known_skips`); `variant.comparisons` (tag `variant`).

Gauges (`|g`): `execs_per_sec`, `workers.alive`, `workers.restarts`,
`crashes`, `hangs`, `memory_kills`*, `server_restarts`*, `worker_crashes`*,
`signatures`, `crash_signatures`*, `backends.count`*, `backends.max_rss_mb`*,
`backends.total_rss_mb`*; per worker (tag `worker`): `worker.execs_per_sec`,
`worker.restarts`, `worker.rss_mb`, `worker.backend_rss_mb`*.

Distributions from workers (`|d`, sampled): `input.ms` (1 in 10, tag
`oracle`), and 1 in 50: `input.queries`, `input.joins`, `input.subqueries`,
`input.aggregate_queries`, `input.grouped_queries`, `input.compound_queries`,
`input.cte_queries`, `input.ordered_queries`, `input.max_sources`,
`input.nodes`, `input.statements`, `input.final_tables`, `input.final_rows`
(ephyra-postgresql's `probe_metrics`).

Not sent (no PostgreSQL equivalent in v1): `coverage.counters`,
`targeted.*`, `profile.*`, `mutation.fallbacks`.

## Testing it locally

```bash
cargo build --release --bin runner
cargo test --release --bin runner          # signatures, shrinking, log parsing
python3 -c 'import socket;s=socket.socket(2,2);s.bind(("127.0.0.1",18125))
while 1: print(s.recv(65535).decode(),flush=True)' > /tmp/statsd.txt &
target/release/runner run --dir /tmp/pgrun-test --workers 4 --hours 0.25 --statsd 127.0.0.1:18125
```

Then: `column -t /tmp/pgrun-test/stats.tsv` grows a line a minute;
`kill -9 $(cat /tmp/pgrun-test/workers/1/pid)` (worker-crash record,
restart at the next input); `kill -9 <a worker's backend>` (from
`workers/K/stats`'s `backend=`: a `signal-9` crash finding naming that
worker, recovery in about a second, all workers restarted); `kill -STOP` a
worker with `--stale-secs 25` (hang record, backend terminated); a psql
query that allocates (`SELECT array_length(array_agg(repeat(g::text,100000)),1)
FROM generate_series(1,20000) g`) with `--max-rss-mb 400` (memory kill,
`server_restart`, no crash finding); kill the supervisor and start it again
with the same command (it resumes, workers carry on from their inputs);
`touch /tmp/pgrun-test/stop` (workers stop, the cluster is stopped).

The finding path without a PostgreSQL bug: start the supervisor with
`EPHYRA_RUNNER_INJECT=norec` (workers inherit it): every norec pass on an
input with a query becomes a failure, shrunk to one query
(`EPHYRA_RUNNER_SHRINK_SECS=30` shortens it) and recorded under
`norec-injected_failure__test_hook_EPHYRA_RUNNER_INJECT_`. Never set it in
a real run.

Clean up: `pg_ctl -D /tmp/pgrun-test/pg/data stop` if a run was killed, then
remove the directory.

## Known gaps (for deployment)

- Deploy scripts (PostgreSQL build, start, watchdog, nightly master
  refresh, cron) are in `deploy/` (`deploy/README.md`).
- Pre-triage (`deploy/PRETRIAGE.md`) gives each finding a verdict against
  master and per-bug patched builds; the patched builds for the planner
  use-after-free and TOAST bugs need their upstream patches fetched first.
- `target` records the checkout's commit, so a rebuild in place under a
  running run is not noticed; `deploy/` builds each master commit in a
  directory of its own and starts a new run on it instead.
- Memory limits are by polling (500 ms): a query growing at >1 GB/s
  overshoots by up to about a gigabyte. On Linux, an `RLIMIT_AS` or cgroup
  memory limit on the postmaster would make such a query fail with "out of
  memory" instead of crashing the cluster's sessions; `deploy/watchdog.sh`
  sets `ulimit -v` (`MEM_LIMIT_MB`) on the supervisor and so on the
  cluster.
- One cluster per run: a crash or memory kill restarts every worker's
  session, so one crashing input costs all workers their input in flight
  (recorded, not lost).
- A timed-out or crashed input is not shrunk; a crash's culprit input is
  only rebuilt (from its seed), not minimized.
- Hang detection works per input (the heartbeat is written between inputs
  and every 10 s while shrinking): an input that legitimately runs past
  `--stale-secs` is killed as a hang.
