# Pre-triage

Most findings of a long run are a known PostgreSQL bug, a false positive
of a stale runner binary, or a failure that needs what a single-session
replay lacks. Pre-triage sorts them unattended, so that only unexplained
findings need a person. The model is `../../../sqlite/coverage`'s pre-triage
(see its deploy/README.md): each known bug has a fix, a check, and the
check's correct output; findings are replayed on master and on one build
per known bug with its fix applied.

Files (all in `deploy/`):

| File | What |
| --- | --- |
| `known-bugs.json` | the registry: per bug its id, candidate folder, signatures, fix (patch path or how to get it), check SQL and its correct output, how it reproduces, and which generator or runner exclusion covers it |
| `build-patched.sh` | one PostgreSQL per open bug with its patch applied, beside plain master |
| `pretriage.py` | verdicts for new findings; `--check-bugs`; `--expected` |
| `pretriage.sh` | the cron wrapper (lock, nice, budget, log) |
| `test_pretriage.py` | unit tests: fake findings, replays and build outputs; `python3 -m unittest` in `deploy/` |

## Builds

`build-patched.sh [--print] [--src DIR] [--ref REF] [--master-prefix DIR]
[--out DIR] [--with-optional] [--only BUG]...` reads `known-bugs.json`
and builds into `OUT/<commit12>/` (OUT: `--out`, else `PG_PATCHED`, else
`PG_BUILDS/patched`; that name starts with no hex digit, so
refresh-master.sh's pruning of `PG_BUILDS/<commit12>` leaves it alone):

- `master/inst`: a link to the master install: `PG_BUILDS/<commit12>/inst`
  (build-pg.sh's, built with `build-pg.sh --no-fetch --ref <commit>` when
  missing) or `--master-prefix`. The commit is `--ref`, else the checkout
  `--master-prefix` belongs to, else `PG_BUILDS/current`'s, else
  `origin/master`.
- `<bug>/inst`: master with the bug's patch, the first available of its
  `sources` (a file under the repository, or the diff of two
  commits of `PG_SRC`). `core` fixes: `git archive` of the commit, the
  patch, `./configure` with master's own `pg_config --configure` (prefix
  replaced), `make install`, contrib amcheck, btree_gin and btree_gist
  (the runner's scripts create them). `contrib` fixes: a copy of the
  master install, with `contrib/<dir>` from `git archive`, patched, built
  and installed into it via PGXS (an install is relocatable). Optional
  bugs (their patch is not saved in this repository) only with
  `--with-optional`.
- Per build: `status` (ok, patch_missing, patch_stale, build_failed,
  skipped), `build.log`, `source`, `fix.patch`, `fix.sha256` (an ok build
  of the same patch is kept; `FORCE=1` rebuilds). `status.json` has them
  all with the commit and its date; `OUT/current` points at the newest
  commit. `--print` shows the plan and whether each source is available,
  and touches nothing.

Times on an M-series Mac: the btree_gist (contrib) build, 7 s; a core
build, a few minutes with `-j` all cores.

## Verdicts

`pretriage.sh` (cron, every 30 minutes, after build-patched.sh has run
once) runs `pretriage.py` on `RUN_ROOT/current` (a run or findings
directory can be given). Each build gets a throwaway cluster for the pass
(initdb under `/tmp/pgptXXXX/`, unix socket only, ports `--port-base`
5641 and up, the runner's settings), a fresh database per replay, and a
watchdog: a client backend over `--max-rss-mb` (4096) is SIGKILLed (the
cluster's sessions restart and it is waited for; the kill and the restart
are not counted as a crash), a replay over `--replay-secs` (180) is
stopped (its backends terminated, then killed).

What is replayed, and when it "fails" (reproduces):

| Finding | Replay | Fails when |
| --- | --- | --- |
| `findings/<signature>/` whose message's failing line is SQL ending in an error (`statement K: ..: msg (SQLSTATE)`, `the original query fails`) | the first example's shrunk `script.sql` with psql (VERBOSITY verbose) | psql shows an ERROR with that SQLSTATE whose message (digits masked) ends that line, or the server crashed |
| any other `findings/<signature>/` (a comparison, the model's rows, a variant that fails), and the above when psql does not reproduce it | `runner replay --finding <example> --pg-dir <the build's cluster>` (the unshrunk input under its oracle; the runner reaches the cluster's socket through a `.s.PGSQL.5432` link) | the runner says failed, a known bug (its `known_error`: the run's binary predates that exclusion) or lost session |
| `findings/crashes/<signature>/` | the newest record's culprit `w<K>.sql` with psql (all in-flight inputs when the log named none), then the oracle on `w<K>.state.txt` | a crash line in the build's server log (TRAP, PANIC, a signal), or a watchdog kill |
| `findings/memory/`, `hangs/` records | the input rebuilt by `runner replay` (against no cluster) as a script, its statement_timeout replaced by `--resource-timeout` (2 s), with psql; then the oracle | a watchdog kill (memory or time) or a crash |
| `findings/timeouts/w<K>-<n>/` | `script.sql` with psql under its own 10 s timeout | a watchdog kill; a timeout honoured on time is `slow-query` |
| `worker-crashes/`, `errors/`, `generator-panics/` | none | `harness` |

The first method that reproduces the finding on master is used on every
patched build. Patched builds are replayed only when master fails.

- `known:<bug>`: fails on master, passes on that bug's patched build.
  When no build passes but the message matches a bug's error pattern and
  that bug has no build (optional, patch missing), `known:<bug>` with
  `basis: signature`.
- `new`: fails on master and on every patched build.
- `not-reproduced`: passes on master, and master is the run's own commit
  (or the run's own build, `PG_BUILDS/<commit12>`, passes too). When the
  message matches a bug that needs concurrency (the TOAST one: another
  database's transaction must hold an older XID), the summary says
  `not-reproduced:needs-concurrency:<bug>` and `reason` explains;
  `generator_drift` when the runner binary's version differs from the
  run's (the oracle replay rebuilds the input from its seed).
- `fixed-on-master`: passes on master, the run's commit is an ancestor of
  master's (`git merge-base` in `PG_SRC`; unknown order counts as older),
  `confirmed` when the run's own build is still there and reproduces it.
- `slow-query`, `harness`, `manual` (could not replay; `reason`).

Written to the finding's `verdict.json` (`findings/<signature>/verdict.json`,
`findings/crashes/<signature>/verdict.json`, `findings/timeouts/<ex>/verdict.json`,
`findings/<memory|hangs|...>/<record>.verdict.json`) with every replay's
outcome, to the run's `pretriage.jsonl`, and as one line in
`RUN_ROOT/deploy.log`:

```
2026-10-14T18:47:51Z pretriage.py: 20261014-test model-statement-INSERT-22008 known:btree-gist-infinite-date (fails on master, passes with the fix of btree-gist-infinite-date)
```

A finding with a verdict is not judged again (`--force` judges it again).

## Known-bug check

`pretriage.sh --check-bugs` runs each open bug's `check` (psql `-X -q -At
-v ON_ERROR_STOP=0 -v VERBOSITY=sqlstate -f -`, fresh database, PATH and
PG* set for `\!` lines, the bug's `limits` enforced by the watchdog) on
master and on its patched build: `open` (the patched build prints
`expected` within the limits, master does not), `fixed-on-master` (master
does: retire the entry), `fix-broken` (the patched build does not),
`open-no-build`, `no-expected`. Result in `RUN_ROOT/known-bugs-status.json`
and a line per bug in `deploy.log`. Run it after every master rebuild.

## The registry today

| Bug | Fix | Local? | Shows as | Reproduces | Exclusion |
| --- | --- | --- | --- | --- | --- |
| `unrecognized-node-10` | pgsql-hackers v2-0001 | no: `obtain` names the archive message (attachments 204590/204591); `parentcheck.diff` (the same pfree guards) stands in; optional | `*-statement-*-XX000`, `*-original-fails-*-XX000`, "unrecognized node type: N" | single session | `sqlancer::known_error` (runner counts it as known) |
| `toast-missing-chunk-vacuum-full` | CF 6824 v7 | as git commits `8e82d6610e7..ce53a55b3b7` after `git fetch` of cfbot `cf/6824` into `PG_SRC`; optional | `*-statement-VACUUM-XX001`, "missing chunk number N for toast value N" | needs concurrency | none |
| `btree-gist-infinite-date` | our `fix.patch` (contrib) | yes | `model-statement-INSERT-22008`, "cannot subtract infinite dates" | single session | `semantics::gist_dates_ok`; oracles skip 22008 |
| `huge-expr-timeout` | our `cfi.patch` | yes | memory kill, hang, `signal-9` crash | single session (resource) | `expr::MAX_DEPTH`; runner memory kill |

Expected outputs: btree-gist's comes from its patched build (`--expected`,
2026-10-14); the others were written from the write-ups' verified outputs
(`expected_from` says so) and should be confirmed with `pretriage.py
--expected <bug>` once their builds exist. On master 158b6d5109 all four
checks print something else (huge-expr: its backend passes the check's
600 MB limit).

### Adding a known bug

1. Its folder under `bugs/` with the patch (or how to get it).
2. An entry in `known-bugs.json`: `fix` (kind, dir for contrib, sources,
   obtain, optional), `signatures` (sqlstate, `errors` regexes over the
   message, `runner_signatures` regexes over the signature directory,
   `finding_kinds`), `reproduce`, `check` (labelled `SELECT 'q1';`
   sections, deterministic output: no OIDs, VERBOSITY sqlstate hides
   messages), `expected: null`, `excluded_by`.
3. `build-patched.sh --only <bug>`, then `pretriage.py --expected <bug>`;
   read the output against the write-up, paste it as `expected`;
   `pretriage.sh --check-bugs` shows `open`.
4. When master gets the fix, `--check-bugs` says `fixed-on-master`: set
   `status: fixed` and `fixed_in` (the entry stays as the record).

## Testing it

```bash
cd case-studies/postgresql/coverage/deploy
python3 -m unittest          # 25 tests, no PostgreSQL
```

End to end on a Mac (2026-10-14; a master checkout `<pg>` installed in
`<pg>/inst`, only read): `build-patched.sh --src <pg> --master-prefix
<pg>/inst --out /tmp/pgpt-builds/patched --only
btree-gist-infinite-date` (7 s), then `pretriage.py --root /tmp/pgpt-run
--builds /tmp/pgpt-builds/patched/current --max-rss-mb 600` over a
hand-made run: a `model-statement-INSERT-22008` finding whose script is
btree_gist's minimal.sql got `known:btree-gist-infinite-date` (master:
the 22008 error; patched: none); a TOAST-like `plans-statement-VACUUM-XX001`
got `not-reproduced:needs-concurrency:toast-missing-chunk-vacuum-full`
(psql and oracle replays both pass); a `signal-9` crash whose script is
the nested-BETWEEN query got `new` (killed at 600 MB on both builds, no
huge-expr build; `matches` names huge-expr-timeout by kind); a generator
panic, `harness`. `--check-bugs`: btree-gist `open`, the others
`open-no-build`.

## Gaps

- Wrong-result findings are judged by the oracle on the unshrunk input,
  rebuilt by the current generator: a generator change since the run
  makes it another input (`generator_drift`).
- Resource findings: a patched build that honours the timeout but still
  allocates past `--max-rss-mb` before it fires is judged as failing; a
  runaway in an oracle's variant query is only seen by the oracle replay,
  under the runner's 10 s timeout.
- One verdict per signature (its first example); later examples are not
  replayed.
- No DogStatsD metric or `events.jsonl` event for verdicts yet (the
  SQLite one sends `triage.verdicts`).
