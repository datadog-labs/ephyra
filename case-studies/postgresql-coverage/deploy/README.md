# Running the PostgreSQL fuzz runner unattended

The runner (`src/bin/runner`, guide in [../RUNNER.md](../RUNNER.md)) runs
for days on a Linux host against PostgreSQL master built with assertions.
These scripts build PostgreSQL, start runs, keep them alive across
restarts, and move them to master's newest commit nightly. They follow
[../../sqlite-coverage/deploy/](../../sqlite-coverage/deploy/) (same layout,
same watchdog and cron pattern). Everything a run produces stays in its
directory (`run.json`, `events.jsonl`, `snapshots.jsonl`, `stats.tsv`, the
cluster `pg/`, `findings/`); metrics go to a DogStatsD listener when one is
configured (optional).

## Files

| File | What it does |
| --- | --- |
| `runner.env.example` | Settings: where runs live, workers, hours, campaign and tag, DogStatsD, memory limits (`MAX_RSS_MB`, `MEM_LIMIT_MB`), heartbeat limit, where the PostgreSQL clone and builds live. Copy to `runner.env`. |
| `build-pg.sh` | Fetches master into `PG_SRC` (cloned when missing) and builds it in a git worktree `PG_BUILDS/<commit12>/`, installed with contrib in its `inst/` (`--coverage`: the instrumented qscore build, `<commit12>-cov/`). `--print` says what it would do and touches nothing. |
| `build-fixed.sh` | Commits master with the fix of every open bug of `known-bugs.json` applied (branch `fuzz/fixes-<master12>` of `PG_SRC`; skips and logs a fix that does not apply), builds it with `build-pg.sh` and points `PG_BUILDS/fixed` at it, with `fixes.json` listing what was applied. With `PG_TARGET=fixed`, runs test this build instead of plain master. |
| `start-run.sh` | Builds the runner, makes `RUN_ROOT/<date>-postgresql-<commit12>/` with a copy of the binary and its arguments (`launch.args`, the PostgreSQL binaries pinned), points `RUN_ROOT/current` at it, starts it through the watchdog. |
| `watchdog.sh` | From cron at boot and every 5 minutes: restarts the current run's supervisor when it is gone (not when the run was stopped or has finished), under the memory limit. The run resumes from its own state. |
| `refresh-master.sh` | Nightly: when master has a new commit, builds it, runs ephyra-postgresql's tests against it, and (when they pass) stops the current run and starts a new one on it; falls back to the old build when the new run does not come up; then removes old builds. |
| `crontab.example` | The cron lines. |
| `known-bugs.json`, `build-patched.sh`, `pretriage.py`, `pretriage.sh`, `test_pretriage.py` | Pre-triage ([PRETRIAGE.md](PRETRIAGE.md)). |

`RUN_ROOT/deploy.log` has a line per thing the scripts did.

## Setting up a host

1. A Linux host (16 vCPUs and 64 GB were used; x86 or arm). PostgreSQL
   refuses to run as root: run everything as an ordinary user.

2. Packages to build PostgreSQL (and the coverage build's clang), and Rust:

   ```bash
   sudo apt-get update
   sudo apt-get install -y build-essential bison flex perl pkg-config \
     libssl-dev zlib1g-dev liblz4-dev libzstd-dev clang rsync git
   command -v cargo || curl --proto '=https' -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
   ```

   (ICU and readline are configured out; `--with-lz4` is needed because
   the generator sets lz4 column compression.)

3. A checkout of this repository: the scripts find everything from their
   own location (`case-studies/postgresql-coverage/deploy/`), with
   `ephyra-postgresql` beside `postgresql-coverage` and the library at
   `crates/ephyra` two levels up. Clone it, or copy a working tree:

   ```bash
   rsync -a --exclude target --exclude .pg --exclude .ephyra \
     <checkout>/ <host>:ephyra/
   ```

   (`.pg` is a local cluster; never copy it.)

4. Settings and the first PostgreSQL build (about 5 minutes on 16 cores;
   the clone is blobless):

   ```bash
   cd <checkout>/case-studies/postgresql-coverage/deploy
   cp runner.env.example runner.env    # adjust WORKERS to the cores
   ./build-pg.sh --print               # what it will do
   ./build-pg.sh                       # ~/pg-builds/<commit12>/inst, and ~/pg-builds/current
   ```

   The first build becomes `PG_BUILDS/current`, which new runs use. Check it
   against ephyra-postgresql's tests (what the nightly refresh does):

   ```bash
   cd <checkout>/case-studies/ephyra-postgresql
   EPHYRA_PG_BIN=$HOME/pg-builds/current/inst/bin EPHYRA_PG_DIR=$HOME/fuzz-runs/t \
     sh -c 'cargo test --lib && cargo test --test cases'
   ~/pg-builds/current/inst/bin/pg_ctl -D ~/fuzz-runs/t/data stop; rm -rf ~/fuzz-runs/t
   ```

   The coverage build (for qscore, not used by the runner) is
   `./build-pg.sh --coverage` (clang; `libclang-rt-<version>-dev` too if
   the link cannot find the profile runtime).

5. Metrics (optional): the runner sends DogStatsD over UDP to `STATSD`
   (`runner.env`; empty for none). Any DogStatsD-compatible listener on
   that port receives them (a Datadog agent with
   `DD_DOGSTATSD_NON_LOCAL_TRAFFIC=true` when it runs in a container, or a
   StatsD server that accepts the DogStatsD tag syntax). The metric names
   and tags are in [../RUNNER.md](../RUNNER.md). A log agent can also follow
   each run's `events.jsonl` (one JSON event per line, `kind` field).

6. Cron and the first run:

   ```bash
   crontab -e                          # paste crontab.example, with the checkout's path
   ./start-run.sh
   column -t ~/fuzz-runs/current/stats.tsv   # a line a minute
   ```

## Memory

The supervisor polls RSS every 500 ms and kills a worker or client backend
over `MAX_RSS_MB` (a `memory_kill` record; the postmaster then restarts
every session). A runaway parse can allocate more than 1 GB/s, so polling
alone overshoots by up to a gigabyte, and several at once can take the
host. On Linux the watchdog therefore starts the supervisor under
`ulimit -v $MEM_LIMIT_MB` (RLIMIT_AS, default 6144 MB): the limit is
inherited by the workers and by the cluster the supervisor starts (pg_ctl,
the postmaster, every backend, since backends are forked), so such a query
fails in `malloc` and PostgreSQL reports "out of memory" (SQLSTATE 53200)
instead. That surfaces as an oracle failure of the input (a finding with
that SQLSTATE in its signature), which is worth a look rather than noise.
The limit is per process and on address space, not RSS: it is above
`MAX_RSS_MB` (plus the 256 MB of shared buffers every backend maps), so
slow growth still ends in a `memory_kill` record. macOS does not enforce
RLIMIT_AS, so the watchdog skips it there. A systemd scope or cgroup
memory limit on the whole run would bound the sum as well, but depends
on the host's systemd and cgroup delegation; not used. The watchdog
also sets `ulimit -c 0`: an assertion failure aborts a backend, and the
crash record already has the log.

## Nightly refresh

`refresh-master.sh` (cron, 05:30): `build-pg.sh` fetches and builds master
(`~/fuzz-runs/refresh-build.log`); when the commit is the current build's
it stops (`FORCE=1` goes on). Else it runs ephyra-postgresql's `cargo test
--lib` and `cargo test --test cases` with `EPHYRA_PG_BIN` pointing at the
new build and a cluster of its own (`RUN_ROOT/t-<commit12>`, removed
afterwards; log `RUN_ROOT/refresh-<commit12>-tests.log`). When they fail,
the run stays on the old build and `deploy.log` names the failures (a
master change the generator or model must follow, or a bug; either would
flood the run): fix or exclude, then `FORCE=1 ./refresh-master.sh`, or set
`SWITCH_ON_TEST_FAILURE=1` to move anyway. When they pass, it stops the
current run (its `stop` file, up to 10 minutes), points `PG_BUILDS/current`
at the new build, starts a new run, and waits up to 5 minutes for it to be
up (supervisor alive, `pg_isready`, a `worker_start` event). If it is not,
it goes back to the old build and starts a run on it. Only then are builds
beyond the newest `KEEP_BUILDS` (3) removed, never the current one, so the
previous run's findings can still be replayed on the build that found them
(`--pg-bin`).

With `PG_TARGET=fixed` in `runner.env` runs test master with the known
bugs' fixes applied (`build-fixed.sh`; refresh-master.sh rebuilds it on each
new master after the tests pass, keeps the previous one when that fails,
and never removes the build `PG_BUILDS/fixed` points at), so they stop
rediscovering bugs that are already reported. Pre-triage still replays on
plain master and the single-fix builds; a finding that only the fixes build
shows is `fixes-only`.

A run never changes binaries under itself: `start-run.sh` pins the
resolved build in `launch.args`, and the run's `target` tag is that build's
commit.

## Watching it

- Metrics (with `STATSD`): `ephyra.postgresql.*`, tagged `run`, `campaign`,
  `work_tag`, `version`, `target:postgresql-<commit12>` and `TAGS` (and
  `oracle`, `variant`, `reason`, `worker`); the list is in
  [../RUNNER.md](../RUNNER.md). Worth alerting on: no metrics, throughput
  dropped, a new failure signature (`signatures`), `crashes`, `hangs`,
  `memory_kills`.
- Without metrics: `stats.tsv` and `events.log` below.

## Operating

- What is running: `column -t ~/fuzz-runs/current/stats.tsv`, the last
  lines of `~/fuzz-runs/current/events.log`, `~/fuzz-runs/deploy.log`.
- Stop a run: `touch ~/fuzz-runs/current/stop` (the watchdog leaves a stopped
  run alone; workers get 90 s, then the cluster is stopped). Start a new
  one: `./start-run.sh`. To resume a stopped run instead, remove `stop` and
  run `./watchdog.sh`.
- Findings: `~/fuzz-runs/current/findings/<signature>/` (`finding.json`,
  `count`, up to 20 examples `w<K>-<n>/` with `message.txt`, the shrunk
  `script.sql` and `input.txt`, the unshrunk `state.txt`); server crashes in
  `findings/crashes/<signature>/<time>/crash.txt` (log excerpt, every
  worker's input in flight as `w<K>.sql`); `hangs/`, `memory/`,
  `worker-crashes/`, `errors/`, `timeouts/`.
- Replay: `psql -X -h ~/fuzz-runs/current/pg -U postgres -d postgres -c
  'CREATE DATABASE r1'` then `psql -X -h ~/fuzz-runs/current/pg -U postgres
  -d r1 -f script.sql` while the run's cluster is up; or
  `~/fuzz-runs/current/runner replay --finding <example dir or state file>
  --out /tmp/r1` (starts the run's cluster when it is stopped; `--pg-bin
  ~/pg-builds/<commit12>/inst/bin` for another build, master's newest for
  "is it fixed"). Before reporting anything, check PostgreSQL master and
  the mailing-list archives.
- A stopped cluster of an old run: `pg_ctl -D <run>/pg/data stop`; old run
  directories can be removed once their findings are triaged.
- The scripts never commit to this repository (`build-fixed.sh` commits
  only to local branches of `PG_SRC`).

## Pre-triage

See [PRETRIAGE.md](PRETRIAGE.md): `known-bugs.json` (the known PostgreSQL
bugs, their patches and checks), `build-patched.sh` (one build per bug with
its patch), `pretriage.py` and its cron wrapper `pretriage.sh` (a verdict
per finding: known, new, not reproduced, fixed on master),
`test_pretriage.py`.
