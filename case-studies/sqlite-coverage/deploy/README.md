# Running the fuzz runner unattended

The runner (`src/bin/runner`) runs for days on a Linux host. These scripts
start runs, keep them alive across restarts, and move them to SQLite trunk's
newest check-in nightly. Everything a run produces stays in its directory
(`run.json`, `events.jsonl`, `snapshots.jsonl`, `findings/`, see
`src/bin/runner/records.rs`); metrics go to a DogStatsD agent when one
listens.

## Files

| File | What it does |
| --- | --- |
| `runner.env.example` | Settings: where runs live, workers by loop profile (`PROFILES`, below), hours, campaign and tag, DogStatsD, `SQLITE_TMPDIR`. Copy to `runner.env`. |
| `start-run.sh` | Builds the runner, makes `RUN_ROOT/<date>-sqlite-<check-in>/` with a copy of the binary and its arguments, points `RUN_ROOT/current` at it, starts it. |
| `watchdog.sh` | From cron at boot and every 5 minutes: restarts the current run's supervisor when it is gone (not when the run was stopped or has finished). The run resumes from its own state. |
| `refresh-trunk.sh` | Nightly: when trunk has a new check-in, re-vendors it into `case-studies/vendor/libsqlite3-sys-trunk/` (with `SQLITE_TARGET=fixed`, build-fixed.sh's build of it; old files kept in `backup-<check-in>/` there), regenerates `sections.tsv`, runs ephyra-sqlite's tests against it, builds the pre-triage CLIs from the same tree and checks the known bugs on it, pre-triages the current run with them, stops the current run and starts a new one. |
| `sections.py` | Regenerates `../sections.tsv` from the vendored amalgamation. |
| `known-bugs.json` | The known SQLite bugs (`case-studies/bugs/sqlite/sqlite-*`): each one's fix and patch options, its check and the check's correct output. The one list the next three read. |
| `build-fixed.sh` | Builds trunk with the fix of every open known bug applied (each bug's `fix-trunk.diff` when its folder has one, else its fix; skips and logs a fix that does not apply) into `RUN_ROOT/fixed/<check-in>/` (amalgamation, CLI, `fixes.json` naming what was applied and skipped), and the CLI into `RUN_ROOT/clis/<check-in>/fixed/`. With `SQLITE_TARGET=fixed`, refresh-trunk.sh vendors this build instead of plain trunk. `--print` lists the fixes and, given a tree, whether each applies. |
| `build-clis.sh` | Builds `RUN_ROOT/clis/<check-in>/trunk/sqlite3` and one `<bug>/sqlite3` per open known bug with its fix applied, from a SQLite tree (default: the vendored check-in's tarball); points `RUN_ROOT/clis/current` at the newest. |
| `pretriage.sh`, `pretriage.py` | From cron every 30 minutes: replays the current run's new findings with those CLIs and gives each a verdict (below). `pretriage.py --check-bugs` is the nightly known-bug check. `test_pretriage.py` tests it (`python3 -m unittest` in this directory). |
| `gaps.sh`, `gaps.py` | The gap list of a run: its workers' coverage profiles merged and exported with llvm-cov against the run's binary; of the in-scope files (`sections.tsv`), functions never called, and regions never executed and branches taken one way in those called, into `<run>/gaps/gaps.md` and `gaps.json`. |
| `crontab.example` | The cron lines. |

`RUN_ROOT/deploy.log` has a line per thing the scripts did.

## Setting up a host

1. A Linux host with many cores and memory (the default allocation is 15
   workers; the original host had 16 vCPUs and 64 GB). Install clang (it
   compiles the instrumented SQLite and links clang's profile runtime),
   `patch`, `curl`, Python 3 and Rust:

   ```bash
   sudo apt-get update && sudo apt-get install -y clang libclang-rt-20-dev patch curl python3
   command -v cargo || curl --proto '=https' -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
   ```

2. Clone the repository and fetch the vendored SQLite (`case-studies/vendor/`
   is not in the repository). sqlite-coverage finds ephyra at
   `../../crates/ephyra` and ephyra-sqlite at `../ephyra-sqlite`;
   `known-bugs.json` names each bug's `fix.diff` and `check.sql` relative to
   `case-studies/`, so the published `case-studies/bugs/sqlite/` is all
   pre-triage needs:

   ```bash
   git clone <repository> ephyra && cd ephyra
   case-studies/vendor/fetch.sh
   ```

3. Check the build reads SQLite's counters (the control score is about 325
   on seed 0; it varies by a few tenths between runs and builds):

   ```bash
   cd case-studies/sqlite-coverage
   cargo build --release --bin runner --bin score
   target/release/score --seed 0 --cases 10000   # 324.9 on the original host, 70 s
   ```

4. Optionally, a DogStatsD listener on `127.0.0.1:8125/udp` for the
   metrics (`STATSD` in `runner.env`; empty for none). A Datadog agent in
   Docker, with its own API key and site:

   ```bash
   docker run -d --name dd-agent --restart unless-stopped \
     -e DD_API_KEY -e DD_SITE="${DD_SITE:-datadoghq.com}" \
     -e DD_HOSTNAME="$(hostname)" -e DD_ENV=ephyra-sqlite \
     -e DD_DOGSTATSD_NON_LOCAL_TRAFFIC=true \
     -p 127.0.0.1:8125:8125/udp \
     -v /var/run/docker.sock:/var/run/docker.sock:ro \
     -v /proc/:/host/proc/:ro -v /sys/fs/cgroup/:/host/sys/fs/cgroup:ro \
     datadog/agent:7
   ```

   To have each run's events as logs as well, add
   `-e DD_LOGS_ENABLED=true -v $HOME/fuzz-runs:/runs:ro` and a
   `conf.d/ephyra.d/conf.yaml` mounted at
   `/etc/datadog-agent/conf.d/ephyra.d/`:

   ```yaml
   logs:
     - type: file
       path: /runs/*/events.jsonl
       service: ephyra-sqlite
       source: ephyra
   ```

5. Settings and cron:

   ```bash
   cd case-studies/sqlite-coverage/deploy
   cp runner.env.example runner.env   # adjust PROFILES to the cores
   crontab -e                          # paste crontab.example, <checkout> replaced
   ./start-run.sh
   ./build-clis.sh                     # the pre-triage CLIs of the vendored check-in, about 2 minutes
   ./pretriage.sh --check-bugs && cat ~/fuzz-runs/known-bugs-status.json
   ```

## Watching it

- Metrics: `ephyra.sqlite.*`, tagged `run`, `campaign`, `work_tag`,
  `version`, `target` and `TAGS` (and `oracle`; per worker and per loop
  `profile` and `storage`, see "Loop profiles and storage"). Among them
  `ephyra.sqlite.execs_per_sec`, `crashes`, `hangs`, `signatures`,
  `coverage.counters`, `workers.alive`, `worker.restarts`, `worker.rss_mb`,
  `oracle.*`, `profile.*`, `triage.verdicts` (pre-triage, by verdict and
  bug), `known_bugs.fixed` and `known_bugs.patch_stale`.
- Events: one JSON line per run event in `events.jsonl` (`kind`:
  `finding`, `crash`, `hang`, `triage` with its `verdict`,
  `known_bug_fixed`, ...); the logs of step 4 when shipped. What is worth
  an alert: an unexplained finding (verdict `new`, `manual` or
  `fixes-only`), a known bug fixed upstream, a known bug's fix no longer
  applying (`patch_stale`), a worker crash or hang, throughput dropping or
  metrics stopping.

## Operating

- What is running: `cat ~/fuzz-runs/current/stats.tsv | column -t`, or the
  last lines of `~/fuzz-runs/current/events.log`.
- Findings: `~/fuzz-runs/current/findings/<signature>/` (`finding.json`, and
  per example `after.db` + `queries.sql`, which the sqlite3 CLI reproduces:
  `sqlite3 after.db < queries.sql`, and `triage.json`, pre-triage's
  verdict). `~/fuzz-runs/current/triage.md` sums up every signature's
  verdicts and lists the new and manual ones first. Before reporting one,
  check it on the latest trunk and the SQLite bug forum.
- A crash or hang record (`findings/crashes/`, `findings/hangs/`) of a fresh
  draw (`mutant=false` in its state: every `gen` and `gen-deep` input, with
  or without `+wal`) reproduces: `./runner replay --state <the record's
  state> --out DIR` (in a WAL file when the state says `storage=wal`).
- Stop a run: `touch ~/fuzz-runs/current/stop` (the watchdog leaves a stopped
  run alone). Start a new one: `./start-run.sh`.
- The scripts never commit. A refresh leaves `sections.tsv` changed in the
  checkout, and the run's header records the repository as dirty.

## Loop profiles and storage

Each worker runs one loop (`src/bin/runner/profiles.rs`): a profile (how
inputs are drawn or mutated, which oracles run) on a storage. `PROFILES` in
`runner.env` assigns them in order, `NAME:COUNT,..`; the default, 15
workers:

```
PROFILES="gen:5,gen-deep:2,rare:2,novel:3,near-miss:1,gen+wal:1,novel+wal:1"
```

- **`NAME+wal`** is profile `NAME` with the case's database in a WAL file
  instead of in memory (ephyra-sqlite's `sqlancer::Storage::Wal`): the
  case's bytes written to a fresh file, opened through the unix VFS in WAL
  mode, a small page cache and autocheckpoint (pages spill to the WAL,
  commits checkpoint), and for half the inputs a close and reopen that
  recovers the WAL index; all chosen from a hash of the case, so a replay
  is the same. An in-memory database never runs the pager's file paths,
  `wal.c` or `os_unix.c`; a local 3-minute smoke run (2026-10-11, macOS)
  had its two WAL workers reach 1817 in-scope regions and 120 functions
  the three memory workers did not (wal.c 4/61 -> 57/61 functions, os_unix.c
  10% -> 37% of regions, pager.c 61% -> 71%). A WAL loop's oracles rotate
  over the seven plus `storage`, which compares every query in memory and
  in the WAL file. A WAL input costs about twice a memory one.
- **Where the files go.** Each WAL worker has a directory
  `ephyra-wal-<run>-w<id>/` under `SQLITE_TMPDIR`, or `/tmp` when that is
  unset or unusable. It should be a tmpfs: every input writes a database,
  its `-wal` and its `-shm`. On the original host `/tmp` was a tmpfs with
  31 GB free and `SQLITE_TMPDIR`; `/dev/shm` was only 64 MB (a container's
  default) and not used. A worker deletes each input's files at the next
  one, empties its directory when it starts (what a killed process left)
  and removes it when it stops. A full disk or an I/O error is never
  reported as a SQLite failure: the storage oracle skips such a comparison
  and counts it (`ephyra.sqlite.oracle.skips{reason:io_skips}`).
- **Tags.** Workers' and loops' metrics, snapshots, findings
  (`finding.json`, `example.json`) and crash and hang records carry
  `profile` (the loop, `gen+wal`) and `storage` (`memory` or `wal`); the
  state file says `storage=wal`, so `runner replay` runs a WAL loop's crash
  or hang in a WAL file again. Datadog turns the `+` of a tag value into
  `_`: it shows `gen_wal` and `novel_wal`.
- **Retired:** `havoc` and `splice` left the default allocation on
  2026-10-11 (they stay selectable): per-profile unique coverage barely
  separates the loop profiles (every profile reached at least 99.8% of the
  union's counters in run 20261011-0438), so the scoreboard is bug
  rediscoveries per profile, from pre-triage's verdicts
  (`ephyra.sqlite.triage.verdicts` by `profile`: `known` and `new`).

## Pre-triage

Most findings are a known SQLite bug, or a false positive of a stale
binary. Pre-triage tells them apart unattended, so that only unexplained
findings need a look.

- **CLIs.** `build-clis.sh [SRC_TREE]` builds, from one SQLite tree, a
  trunk CLI and one CLI per open bug of `known-bugs.json` with its fix
  applied (`patch -p<strip> -d <dir>`), each in a fresh copy configured
  inside itself, with the runner's own SQLite options, in parallel, into
  `RUN_ROOT/clis/<check-in>/` (`status.json`; per CLI `status`: ok,
  patch_stale or build_failed, and `build.log`). The trunk CLI is the
  vendored check-in: the nightly refresh builds them from the tree it
  vendors from; with no argument the script downloads the vendored
  check-in's tarball. A CLI built from the same fix is kept, so adding a
  bug rebuilds only it (`FORCE=1` rebuilds all).
- **Verdicts.** `pretriage.sh` (cron, every 30 minutes; flock, nice, a
  timeout per statement, a time budget per pass) runs `pretriage.py` on the
  current run (`--all`: every run; a findings directory can be given
  directly), with `--bugs-root` the `case-studies/` directory. Every
  example without a `triage.json` is replayed with the
  CLIs on copies of its `after.db`, the two sides of the oracle's
  comparison compared as the oracle compares them (JSONB by `json(x)`; a
  side the runner computed from other queries' rows, setops-def's and
  window-def's, recomputed from each CLI's rows of them), and
  gets one of: `excluded` (an error the current oracles skip: a stale
  binary; or a case's statement refused as "non-deterministic use of
  julianday() in an index", which they skip and count as
  `nondeterministic_skips`), `jsonb-equivalent` (the message's own results agree once JSONB
  compares as JSON text), `known` with the bugs whose fix makes it go away
  (`known:<bug>` in summaries; a bug without a fix is matched by its error
  text and SQL patterns), `new` (trunk reproduces it, no fix explains it),
  `fixed-upstream` (trunk no longer reproduces it, the run's target did),
  `not-reproducible` (the run's own check-in does not reproduce it),
  `fixes-only` (a fixed run's only: its own SQLite, trunk with the known
  bugs' fixes, reproduces it, plain trunk does not; below), or
  `manual`. The exact rules are `pretriage.py`'s module documentation.
  A WAL worker's finding is replayed on a copy of `after.db` put in WAL
  mode first; a storage oracle's (`results differ (storage)`) with its
  original in memory (the CLI's `.open --deserialize`) and its variant on
  a WAL-mode copy, judged as any comparison. `after.db` is the database
  after the case's statements in memory, for WAL workers too, so a failure
  that needs the WAL's own frames is not reproduced (`not-reproducible`;
  `example.json` keeps the case's `wal_settings` for a manual replay).
  A plans oracle's `plans-noopt` finding (the query as written against
  itself with SQLite's optimizations off, `SQLITE_TESTCTRL_OPTIMIZATIONS`;
  named `plans-noopt-<optimization>`, e.g. `plans-noopt-BloomFilter`, when
  that one alone off reproduces it, so each culprit is a signature of its
  own) ends with the mask its variant ran under (`  optimizations off:
  0x00080000 (BloomFilter)`); its variant is replayed after `.testctrl
  optimizations <mask>`, on trunk and on each patched CLI, so a known bug's
  fix explains it as any other comparison. By hand: the query, then
  `.testctrl optimizations 0x80000` and the query again, in one CLI
  session (the CLI prints the optimizations it left on).
  Each verdict is written to the example's `triage.json`, the run's
  `triage.jsonl`, a `triage` event in its `events.jsonl` (so a log) and the
  counter `ephyra.sqlite.triage.verdicts` (tags verdict, bug, oracle,
  explained, the example's profile and storage, and the run's tags);
  `triage.md` is rewritten every pass.
- **Nightly bug check.** When trunk moves, `refresh-trunk.sh` builds the
  new CLIs from the tree it vendored, runs `pretriage.py --check-bugs`
  (each open bug's check on the new trunk CLI: its exact correct output
  means fixed upstream, event `known_bug_fixed` and
  `ephyra.sqlite.known_bugs.fixed`; a fix that no longer applies,
  `patch_stale`; all in `RUN_ROOT/known-bugs-status.json`), pre-triages the
  old run's remaining findings with the new CLIs (fixed upstream ones show),
  and pre-triages the new run after starting it.

### The fixed target (`SQLITE_TARGET=fixed`)

A run on plain trunk spends much of its time rediscovering the known bugs.
With `SQLITE_TARGET=fixed` in `runner.env` the runner tests trunk with every
open known bug's fix applied instead, so that what it finds is new:

- **Build.** On each new trunk, `refresh-trunk.sh` runs `build-fixed.sh` on
  the trunk tree it downloaded: a fresh copy of the tree's files (the
  manifest's; configure writes the tree's absolute path into its Makefile,
  so the copy is configured inside itself), each open bug's fix applied in
  the registry's order after a dry run (the folder's `fix-trunk.diff` when
  there is one, which a bug without a registry fix can have too, else
  `fix.path`; the registry's `-p<strip> -d <dir>`, else `-p1` or `-p0` from
  the top), then `make sqlite3.c shell.c` and the CLI with the runner's
  options. A fix that does not apply is skipped and logged; one that applies
  reversed is "already in the tree" (trunk has fixed the bug: retire it).
  The amalgamation's `SQLITE_SOURCE_ID` keeps the check-in and ends in
  `alt1`, SQLite's mark of a modified tree, so the run's target and its
  CLIs' directory stay the check-in.
- **Vendoring.** refresh-trunk.sh vendors that amalgamation (and its
  `fixes.json`) into `case-studies/vendor/libsqlite3-sys-trunk/sqlite3/`
  instead of plain trunk's, with the same `backup-<check-in>/` (fixes.json
  included); `sections.tsv` and ephyra-sqlite's tests follow the vendored
  files. A fail-when-fixed test of ephyra-sqlite fails when one of our
  fixes is for its bug (sqlite-right-join-using-star-ambiguous's
  `sqlite_rejects_star_over_right_join_using_as_ambiguous`, say): expected,
  logged, not a reason to stay on the old trunk. When the fixed build fails,
  plain trunk is vendored and the log says so. Plain trunk's and each bug's
  CLIs and the nightly bug check are built and run as before.
- **The run.** `start-run.sh` passes `--fixed-bugs <bug>,..` (fixes.json's
  `applied`, when its `sqlite3_c_sha256` is the vendored sqlite3.c's), which
  the runner records as `config.fixed_bugs` in run.json and its
  `run_start` event; its deploy.log line names them, and it warns when
  `SQLITE_TARGET` and the vendored files disagree.
- **Pre-triage** replays on plain trunk and the single-fix CLIs as for any
  run, and also on the run's own SQLite, `RUN_ROOT/clis/<check-in>/fixed/
  sqlite3`: a finding trunk does not reproduce and the fixed CLI does is
  `fixes-only` (a bug in one of our fixes, or one they expose together;
  explained:false, so it counts like `new` and is under "Needs a look");
  the plain target reproducing it too (an older check-in) makes it
  `fixed-upstream`, the fixed CLI not reproducing it `not-reproducible`. A
  known bug whose fix the run has should no longer come up; when one does,
  it stays `known` with `fix_incomplete` and a `note` (the fixed CLI
  reproducing it means the fix is incomplete).
- **Switching** (to or from fixed) takes effect at the next trunk move;
  `FORCE=1 deploy/refresh-trunk.sh` does it now. A bug added to the
  registry, or a new `fix-trunk.diff`, reaches the fixed build at the next
  trunk move too (or `FORCE=1`). By hand, without touching the vendored
  files: `deploy/build-fixed.sh --print /path/to/sqlite` lists what applies,
  `deploy/build-fixed.sh --out DIR /path/to/sqlite` builds into DIR (the
  CLI still goes to `RUN_ROOT/clis/<check-in>/fixed/`; point `RUNNER_ENV`
  at a scratch env file for a scratch `RUN_ROOT`).

Cron line (in `crontab.example`):

```
*/30 * * * *  <checkout>/case-studies/sqlite-coverage/deploy/pretriage.sh
```

### Adding a known bug

1. Make its folder `case-studies/bugs/sqlite/sqlite-<name>/` (beside the
   others) with a write-up, `fix.diff` (against a fresh trunk tree,
   `https://sqlite.org/src/tarball/trunk/sqlite.tar.gz`) and `check.sql`: a
   few labelled (`SELECT 'q1';`) self-contained queries with deterministic
   output (ORDER BY, `typeof()`, `quote()`; no `.testctrl`) whose output on
   trunk differs from the output with the fix. More than one form of the bug
   guards against a planner change hiding it.
2. Add an entry to `known-bugs.json`, paths relative to `case-studies/`:
   `name`, `folder`, `summary`, `status: "open"`, `fix: {"path", "strip",
   "dir"}` (the options with which `patch -p<strip> -d <trunk tree>/<dir>
   -i <path>` applies it; add a `fix-trunk.diff` beside the folder's own
   when that one does not apply), `repro` (the check.sql), and
   `expected: null` for now. A bug whose fix is
   in the code that writes index entries gets `reindex: true`: its patched CLI
   replays on a copy its own `REINDEX` rebuilt (after.db's indexes were written
   by the run's unpatched SQLite), and the bug is credited only if trunk still
   reproduces the failure on a copy trunk's `REINDEX` rebuilt. A bug without a
   fix has `fix: null` and `match: {"error", "sql_all"[, "sql_any"]}`
   instead (`error: null` for a wrong result: the SQL patterns alone then
   say which reproduced failure it is), and its `expected` written by hand
   (the check's output with the optimization at fault turned off, say).
3. On the host: `deploy/build-clis.sh` (builds only the new CLI), then
   `python3 deploy/pretriage.py --expected sqlite-<name>` prints the check's
   output on the patched CLI. Read it against the write-up, paste it as
   `expected`, and run `deploy/pretriage.sh --check-bugs`: the bug is
   still-wrong, and `patched_matches_expected` is true in
   `~/fuzz-runs/known-bugs-status.json`.
4. Findings already triaged keep their verdict; to triage a run again with
   the new bug, `deploy/pretriage.sh --force` (or remove the `new`
   examples' `triage.json`).

When the nightly check reports a bug fixed upstream, retire it: set its
status to `fixed` and record `fixed_in` (`checkin`, `how` it reached trunk,
the `trunk` checked; nothing builds, matches or checks it then, and
`known-bugs-status.json` lists it under `retired`), and keep the entry.
Retired on 2026-10-13, fixed on trunk f69fd5f5c3 and the runner's
bfe473ab60f6: sqlite-intersect-full-join-cte (e6e27e37eb),
sqlite-groupby-subquery-orderby-direction (1b574df8c7),
sqlite-expr-index-constant-propagation (fee60d93e9),
sqlite-json-valid-stale-cache-json5 (ba74c33b6a) and
sqlite-aggregate-orderby-replaces-argument (ffe3f056f1), all from
branch-3.54 by merge 5a05aded7f.
