#!/usr/bin/env python3
"""Pre-triage of the PostgreSQL fuzz runner's findings, against the known
PostgreSQL bugs of known-bugs.json (python3, standard library only).

    pretriage.py [--force] [PATH ...]      # runs or findings dirs (default RUN_ROOT/current)
    pretriage.py --check-bugs              # each known bug's check on master and its patched build
    pretriage.py --expected [BUG ...]      # a bug's check output on its patched build

Builds: build-patched.sh's OUT/current (`--builds`): `master/inst/bin` and
one `<bug>/inst/bin` per open bug whose `status` is ok; `--master-bin`
replaces master, `--build NAME=BINDIR` adds or replaces one. Every build
gets a throwaway cluster of its own for the pass (initdb in a short
directory under /tmp, unix socket only, port `--port-base` + k, the
runner's settings), a fresh database per replay, and a watchdog: a client
backend over `--max-rss-mb` is killed (SIGKILL; the postmaster restarts the
sessions and the cluster is waited for), and a replay past `--replay-secs`
is stopped (psql or the runner killed, backends terminated, then killed).

Findings (`RUN_ROOT/current/findings/`), each judged once (`--force`
again), its verdict written next to it:

- `<signature>/` (an oracle or model failure; `verdict.json` in it), from
  its first example `w<K>-<n>/` with a script.sql and state.txt. Methods,
  the first that reproduces the failure on master is used on every build:
  - `psql`: when the message's failing line is SQL that ends in an error
    (`statement K: <sql>: <error> (SQLSTATE)`, `the original query fails:
    ..`): the example's shrunk script.sql run by psql (VERBOSITY verbose);
    it fails when psql reports an ERROR with the same SQLSTATE whose
    message (digit runs masked) ends the message's line, or the server
    crashed.
  - `oracle`: `runner replay --finding <example> --pg-dir <the cluster>`:
    the unshrunk input rebuilt from its seed under the finding's oracle;
    it fails when the runner's verdict is a failure (the same signature, or
    else noted as `other_signature`), a known bug (the runner's own
    `known_error`, i.e. a run binary older than that exclusion) or a lost
    session. The input is rebuilt by the runner binary's generator: when
    its version differs from the run's (`run.json` version_id) the verdict
    carries `generator_drift`.
- `crashes/<signature>/` (`verdict.json` in it), from its newest record:
  the culprits' `w<K>.sql` (every in-flight input when no culprit is
  known) run by psql; it fails when the server log gains a crash line
  (TRAP, PANIC, terminated by a signal other than the watchdog's) or the
  watchdog kills a backend; then `oracle` on `w<K>.state.txt`.
- `memory/`, `hangs/` records (`<record>.verdict.json` beside each): the
  input rebuilt by `runner replay` into a script (once, against no
  cluster), its `statement_timeout` replaced by `--resource-timeout` (2 s),
  run by psql; it fails when the watchdog kills a backend for memory or
  stops the replay for time, or the server crashes; then `oracle` (the
  variants too, under the runner's own 10 s timeout).
- `timeouts/w<K>-<n>/` (`verdict.json` in it): script.sql by psql under its
  own 10 s timeout: a replay the watchdog had to stop fails; a statement
  timeout honoured on time is `slow-query`.
- `worker-crashes/`, `errors/`, `generator-panics/` records: `harness` (the
  runner's or the generator's side, not PostgreSQL's), not replayed.

Verdicts:

- `known:<bug>`: fails on master, passes on that bug's patched build
  (`bugs` lists every build that passes). When no patched build passes but
  the message matches a known bug's error pattern and that bug has no
  build (optional, patch missing), `known:<bug>` with `basis: signature`.
- `new`: fails on master and on every patched build (`matches` names
  known bugs whose patterns match anyway, weaker evidence).
- `not-reproduced`: passes on master, master is the run's own commit (or
  the run's own build, `--pg-builds`/<commit12>, passes too); `reason`
  says why it may be: a known bug matched by its error that needs
  concurrency (`not-reproduced:needs-concurrency:<bug>` in summaries), the
  rebuilt input drifted, or a failure that needs what a single-session
  replay lacks (other workers' transactions, timing).
- `fixed-on-master`: passes on master, the run's commit is older than
  master's (`git merge-base --is-ancestor` in `--pg-src`; unknown order
  counts as older, noted), and the run's own build reproduces it when it
  is still there (`confirmed`).
- `fixes-only`: passes on master, fails on the run's own build, which is
  build-fixed.sh's master with the known bugs' fixes (it has a fixes.json):
  a bug in one of the fixes, or one a fix exposes. For a crash, also when its
  assertion is in that build's source but not in master's (`basis: source`;
  such a crash often needs concurrency, so a replay may not show it).
- `slow-query`: a timeouts record whose statement timeout fires on time on
  master.
- `harness`, `manual` (nothing to replay, or a replay that could not run;
  `reason` says why).

Each verdict goes to its `verdict.json`, a line of the run's
`pretriage.jsonl`, and one summary line in `RUN_ROOT/deploy.log`.

`--check-bugs`: every open bug's `check` on master and on its patched
build (fresh database each, the watchdog with the bug's `limits`): the
patched output equal to `expected` (and within the limits) and master's
not means `open`; master's equal means `fixed-on-master`; a patched output
that differs, `fix-broken`; master wrong but no patched build,
`open-no-build`; no `expected`, `no-expected`.
All in RUN_ROOT/known-bugs-status.json, a line per bug in deploy.log.
"""

import argparse
import concurrent.futures
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
HARNESS_KINDS = ("worker-crashes", "errors", "generator-panics")
RECORD_FOLDERS = ("crashes", "hangs", "memory", "worker-crashes", "errors", "generator-panics", "timeouts")
CRASH_RE = re.compile(r"TRAP: |PANIC: |terminated by signal (\d+)|exited with exit code (\d+)|all server processes terminated")
PSQL_ERROR_RE = re.compile(r"ERROR:\s+([0-9A-Z]{5}): (.*)")
LOST_RE = re.compile(r"server closed the connection unexpectedly|connection to server was lost|terminating connection|could not connect to server|connection to server .* failed")


# ---------------------------------------------------------------- parsing

def fold(text):
    """Digit runs masked, whitespace squeezed: messages compared across builds
    (`unrecognized node type: 7` against `: 10`)."""
    return " ".join(re.sub(r"\d+", "N", text).split())


def sqlstate_of(line):
    m = re.search(r"\(([0-9A-Z]{5})\)\s*$", line)
    return m.group(1) if m else None


def failing_line(message):
    """The message's line that ends in an error with its SQLSTATE: `(sqlstate,
    line without it)`, or None."""
    for line in message.splitlines():
        code = sqlstate_of(line)
        if code:
            return code, re.sub(r"\s*\([0-9A-Z]{5}\)\s*$", "", line).strip()
    return None


def message_kind(message):
    """`sql-error` (the failing line is SQL that ends in an error, which the
    shrunk script replays), `harness` (a panic in the oracle or generator),
    or `oracle` (anything else: a comparison, the model's rows, a variant
    that fails; only the oracle can judge it)."""
    first = message.splitlines()[0] if message else ""
    if first.startswith("panic: "):
        return "harness"
    if first.startswith("statement ") and sqlstate_of(first):
        return "sql-error"
    if first.startswith("the original query fails: ") and failing_line(message):
        return "sql-error"
    return "oracle"


def psql_errors(output):
    """`[(sqlstate, message)]` of psql's output with VERBOSITY verbose."""
    return [(m.group(1), m.group(2).strip()) for m in PSQL_ERROR_RE.finditer(output)]


def psql_lost(output):
    return bool(LOST_RE.search(output))


def error_matches(expected, errors):
    """Whether psql's errors hold the finding's: the same SQLSTATE and the
    error's message (digits masked) ending the finding's failing line."""
    code, line = expected
    fl = fold(line)
    return any(c == code and fold(m) and fl.endswith(fold(m)) for c, m in errors)


def crash_lines(log, ignore_kill=False):
    """The crash lines of a server log excerpt (the supervisor's). With
    `ignore_kill` (the watchdog killed a backend in it), the SIGKILL and
    the restart it causes are not crashes."""
    out = []
    for line in log.splitlines():
        m = CRASH_RE.search(line)
        if not m:
            continue
        if ignore_kill and (m.group(1) == "9" or "all server processes terminated" in line):
            continue
        if m.group(2) in ("0", "1"):
            continue
        out.append(line.strip())
    return out


def parse_oracle_verdict(text):
    """The runner replay's verdict.txt: `(class, signature, text)`, class one
    of pass, fail, known, timeout, lost, unknown."""
    t = text.strip()
    if t.startswith("passed"):
        return "pass", None, t
    m = re.match(r"failed \(([^)]*)\): ", t)
    if m:
        return "fail", m.group(1), t
    if t.startswith("a known bug"):
        return "known", None, t
    if t.startswith("a statement timeout"):
        return "timeout", None, t
    if t.startswith("the session was lost"):
        return "lost", None, t
    return "unknown", None, t


def pairs(text):
    return dict(l.split("=", 1) for l in text.splitlines() if "=" in l)


# ---------------------------------------------------------------- registry

def load_registry(path):
    with open(path) as f:
        return json.load(f)


def open_bugs(registry):
    return [b for b in registry["bugs"] if b.get("status") == "open"]


def match_bugs(finding, registry):
    """Known bugs whose patterns a finding matches: `[(id, strength)]`,
    strength `error` (an error pattern over its message, or a runner
    signature pattern with the bug's SQLSTATE) or `kind` (only its kind is
    one the bug shows as)."""
    out = []
    msg = finding.message or ""
    for b in open_bugs(registry):
        s = b.get("signatures", {})
        strong = any(re.search(p, msg) for p in s.get("errors", []))
        if not strong and finding.signature:
            strong = any(re.search(p, finding.signature) for p in s.get("runner_signatures", []))
        if strong:
            out.append((b["id"], "error"))
        elif finding.kind in s.get("finding_kinds", []) and finding.kind != "failure":
            out.append((b["id"], "kind"))
    return out


# ---------------------------------------------------------------- findings

class Finding:
    """One thing to judge. kind: failure, crash, memory, hangs, timeouts, or
    a harness kind."""

    def __init__(self, kind, ident, path, verdict_path, message="", signature=None):
        self.kind, self.id, self.path, self.verdict_path = kind, ident, path, verdict_path
        self.message, self.signature = message, signature
        self.example = None       # failure/timeouts: the example dir
        self.scripts = []         # crash: the w<K>.sql to run
        self.states = []          # crash: the w<K>.state.txt
        self.record = None        # memory/hangs: the record file


def read_text(path, default=""):
    try:
        with open(path, errors="replace") as f:
            return f.read()
    except OSError:
        return default


def read_json(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def _examples(d):
    return sorted(e for e in os.listdir(d) if os.path.isdir(os.path.join(d, e)) and os.path.exists(os.path.join(d, e, "state.txt")))


def discover(findings, force=False):
    """The findings of a findings directory that have no verdict yet."""
    out = []
    if not os.path.isdir(findings):
        return out
    for name in sorted(os.listdir(findings)):
        d = os.path.join(findings, name)
        if not os.path.isdir(d):
            continue
        if name not in RECORD_FOLDERS:
            v = os.path.join(d, "verdict.json")
            if os.path.exists(v) and not force:
                continue
            exs = _examples(d)
            f = Finding("failure", name, d, v, signature=name)
            if exs:
                f.example = os.path.join(d, exs[0])
                f.message = read_text(os.path.join(f.example, "message.txt"))
            out.append(f)
        elif name == "crashes":
            for sig in sorted(os.listdir(d)):
                sd = os.path.join(d, sig)
                v = os.path.join(sd, "verdict.json")
                if not os.path.isdir(sd) or (os.path.exists(v) and not force):
                    continue
                times = sorted((t for t in os.listdir(sd) if os.path.isdir(os.path.join(sd, t))), key=lambda t: (len(t), t))
                f = Finding("crash", "crashes/" + sig, sd, v, signature=sig)
                if times:
                    at = os.path.join(sd, times[-1])
                    crash = read_text(os.path.join(at, "crash.txt"))
                    f.message = crash[:4000]
                    m = re.search(r"culprit worker\(s\), whose backend the log names: \[([0-9, ]*)\]", crash)
                    culprits = [int(x) for x in m.group(1).replace(" ", "").split(",") if x] if m else []
                    ws = sorted(int(x[1:-4]) for x in os.listdir(at) if re.fullmatch(r"w\d+\.sql", x))
                    pick = [w for w in ws if w in culprits] or ws
                    f.scripts = [os.path.join(at, "w%d.sql" % w) for w in pick]
                    f.states = [os.path.join(at, "w%d.state.txt" % w) for w in pick if os.path.exists(os.path.join(at, "w%d.state.txt" % w))]
                out.append(f)
        elif name == "timeouts":
            for ex in sorted(os.listdir(d)):
                e = os.path.join(d, ex)
                v = os.path.join(e, "verdict.json")
                if not os.path.isdir(e) or (os.path.exists(v) and not force):
                    continue
                f = Finding("timeouts", "timeouts/" + ex, e, v, message=read_text(os.path.join(e, "message.txt")))
                f.example = e
                out.append(f)
        else:
            for rec in sorted(os.listdir(d)):
                p = os.path.join(d, rec)
                if not rec.endswith(".txt") or rec.endswith(".verdict.json"):
                    continue
                v = p[:-4] + ".verdict.json"
                if os.path.exists(v) and not force:
                    continue
                f = Finding(name, name + "/" + rec[:-4], p, v, message=read_text(p)[:4000])
                f.record = p
                out.append(f)
    return out


# ---------------------------------------------------------------- outcomes and verdicts

class Outcome:
    """One replay on one build. fails: True, False, or None (could not run)."""

    def __init__(self, fails, method, detail="", **extra):
        self.fails, self.method, self.detail, self.extra = fails, method, detail, extra

    def to_json(self):
        d = {"fails": self.fails, "method": self.method, "detail": self.detail[:600]}
        d.update(self.extra)
        return d


class Context:
    """What a verdict needs besides the outcomes."""

    def __init__(self, run_commit=None, master_commit=None, run_older=None, generator_drift=False, run_fixes=False):
        self.run_commit, self.master_commit, self.run_older, self.generator_drift = run_commit, master_commit, run_older, generator_drift
        # The run's target is build-fixed.sh's master with the known bugs'
        # fixes (its build has a fixes.json).
        self.run_fixes = run_fixes
        # For a crash on such a run: its assertion exists only in the
        # fixes build's source (assert_only_in), so master cannot reproduce it.
        self.assert_fixes_only = False


def decide(finding, outcomes, registry, ctx, built=(), target=None, tried=()):
    """The verdict of a finding from its outcomes (`{build: Outcome}`,
    `master` first; patched builds only when master fails), the bugs that
    have a build (`built`), and the run's own build's outcome (`target`,
    when master passes and the run is on another commit)."""
    v = {"finding": finding.id, "kind": finding.kind}
    matches = match_bugs(finding, registry)
    strong = [b for b, s in matches if s == "error"]
    if matches:
        v["matches"] = [{"bug": b, "by": s} for b, s in matches]
    if finding.kind in HARNESS_KINDS or (finding.kind == "failure" and message_kind(finding.message) == "harness"):
        v.update(verdict="harness", reason="the runner's or the generator's side (a worker crash, a runner error, a panic), not PostgreSQL's: not replayed")
        return v
    master = outcomes.get("master")
    if master is None or master.fails is None:
        v.update(verdict="manual", reason="no replay on master: " + (master.detail if master else "nothing to replay"))
        return v
    v["outcomes"] = {k: o.to_json() for k, o in outcomes.items()}
    if len(tried) > 1:
        v["master_tried"] = [o.to_json() for o in tried]
    if ctx.generator_drift and master.method == "oracle":
        v["generator_drift"] = True
    if finding.kind == "timeouts" and not master.fails and master.extra.get("timeout"):
        v.update(verdict="slow-query", reason="the statement timeout fires on time on master: a slow query, not a PostgreSQL failure by itself")
        return v
    if not master.fails:
        if ctx.run_fixes and ctx.assert_fixes_only:
            v["run_commit"], v["master_commit"] = ctx.run_commit, ctx.master_commit
            v.update(verdict="fixes-only", basis="source", reason="its assertion exists only in the run's own build (master with the known bugs' fixes), not in master's source: a bug in a fix, or one a fix exposes")
            return v
        if ctx.run_fixes and target is not None and target.fails:
            v["outcomes"]["target"] = target.to_json()
            v["run_commit"], v["master_commit"] = ctx.run_commit, ctx.master_commit
            v.update(verdict="fixes-only", reason="passes on master, fails on the run's own build (master with the known bugs' fixes): a bug in a fix, or one a fix exposes")
            return v
        differ = ctx.run_commit and ctx.master_commit and ctx.run_commit[:12] != ctx.master_commit[:12]
        if differ and ctx.run_older is not False:
            v["run_commit"], v["master_commit"] = ctx.run_commit, ctx.master_commit
            if target is not None and target.fails is not None:
                v["outcomes"]["target"] = target.to_json()
                if target.fails:
                    v.update(verdict="fixed-on-master", confirmed=True, reason="master passes, the run's own commit reproduces it")
                    return v
                v.update(verdict="not-reproduced", reason="passes on master and on the run's own commit: " + _why_not(finding, registry, strong, ctx, master))
                return v
            v.update(verdict="fixed-on-master", confirmed=False,
                     reason="master passes and the run's commit is older" + ("" if ctx.run_older else " (order unknown)") + "; the run's own build is gone, so not confirmed")
            return v
        need = [b for b in strong if _bug(registry, b).get("reproduce") == "needs-concurrency"]
        if need:
            v["needs"] = "concurrency"
            v["label"] = "not-reproduced:needs-concurrency:" + need[0]
        v.update(verdict="not-reproduced", reason=_why_not(finding, registry, strong, ctx, master))
        return v
    explained = [b for b, o in outcomes.items() if b != "master" and o is not None and o.fails is False]
    if explained:
        v.update(verdict="known:" + explained[0], bugs=explained, basis="replay",
                 reason="fails on master, passes with the fix of " + ", ".join(explained))
        return v
    unbuilt = [b for b in strong if b not in built]
    if unbuilt:
        v.update(verdict="known:" + unbuilt[0], bugs=unbuilt, basis="signature",
                 reason="fails on master and on every patched build; its message matches " + ", ".join(unbuilt) + ", which has no patched build to confirm it")
        return v
    v.update(verdict="new", reason="fails on master and on every patched build (" + (", ".join(sorted(b for b in outcomes if b != "master")) or "none built") + ")")
    return v


def _bug(registry, bid):
    return next((b for b in registry["bugs"] if b["id"] == bid), {})


def _why_not(finding, registry, strong, ctx, master):
    for b in strong:
        bug = _bug(registry, b)
        if bug.get("reproduce") == "needs-concurrency":
            return "needs concurrency: matches known bug %s (%s)" % (b, bug.get("reproduce_note", "a single-session replay cannot show it"))
    if ctx.generator_drift and master.method == "oracle":
        return "the input was rebuilt by a generator other than the run's (generator_drift): it may not be the same input"
    return "a single-session replay on master passes: it may need concurrency (other workers' transactions on the shared cluster), timing, or the cluster's state"


def label(v):
    return v.get("label") or v["verdict"]


# ---------------------------------------------------------------- clusters and the watchdog

def ps_table():
    """`{pid: (ppid, rss_kb, command)}`."""
    out = subprocess.run(["ps", "-ax", "-o", "pid=,ppid=,rss=,command="], capture_output=True, text=True).stdout
    t = {}
    for line in out.splitlines():
        parts = line.split(None, 3)
        if len(parts) >= 3 and parts[0].isdigit():
            t[int(parts[0])] = (int(parts[1]), int(parts[2]) if parts[2].isdigit() else 0, parts[3] if len(parts) > 3 else "")
    return t


class Cluster:
    """A throwaway cluster of one build: unix socket only, on `port`, in a
    short directory; `r/.s.PGSQL.5432` links to its socket for the runner
    (whose sessions connect to port 5432 of EPHYRA_PG_DIR)."""

    def __init__(self, name, bindir, root, port):
        self.name, self.bin, self.port = name, bindir, port
        self.dir = os.path.join(root, name[:24])
        self.data = os.path.join(self.dir, "data")
        self.log = os.path.join(self.dir, "server.log")
        self.rdir = os.path.join(self.dir, "r")
        self.dbs = 0
        self.killed = []           # pids the watchdog killed
        self.lock = threading.Lock()

    def env(self):
        e = dict(os.environ)
        e.update(PGHOST=self.dir, PGPORT=str(self.port), PGUSER="postgres", PATH=self.bin + os.pathsep + e.get("PATH", ""))
        e.pop("PGDATABASE", None)
        return e

    def start(self):
        os.makedirs(self.dir, exist_ok=True)
        if not os.path.exists(os.path.join(self.data, "PG_VERSION")):
            r = subprocess.run([os.path.join(self.bin, "initdb"), "-D", self.data, "-U", "postgres", "--no-sync", "-E", "UTF8", "--locale=C", "-A", "trust"],
                               capture_output=True, text=True)
            if r.returncode:
                raise RuntimeError("initdb: " + r.stderr[-500:])
        opts = ("-k %s -p %d -c listen_addresses= -c fsync=off -c synchronous_commit=off -c full_page_writes=off -c autovacuum=off "
                "-c max_connections=50 -c shared_buffers=256MB -c max_locks_per_transaction=256" % (self.dir, self.port))
        r = subprocess.run([os.path.join(self.bin, "pg_ctl"), "-D", self.data, "-l", self.log, "-o", opts, "-w", "-t", "60", "start"], capture_output=True, text=True)
        if r.returncode:
            raise RuntimeError("pg_ctl start: " + (r.stdout + r.stderr)[-500:] + read_text(self.log)[-500:])
        os.makedirs(self.rdir, exist_ok=True)
        link = os.path.join(self.rdir, ".s.PGSQL.5432")
        if not os.path.lexists(link):
            os.symlink(os.path.join(self.dir, ".s.PGSQL.%d" % self.port), link)

    def stop(self):
        subprocess.run([os.path.join(self.bin, "pg_ctl"), "-D", self.data, "-m", "immediate", "stop"], capture_output=True)

    def postmaster(self):
        try:
            return int(read_text(os.path.join(self.data, "postmaster.pid")).split()[0])
        except (IndexError, ValueError):
            return None

    def ready(self, wait=60):
        """Wait for the cluster to accept connections (after a crash restart);
        restart it when it does not."""
        deadline = time.time() + wait
        while time.time() < deadline:
            r = subprocess.run([os.path.join(self.bin, "pg_isready"), "-h", self.dir, "-p", str(self.port), "-U", "postgres"], capture_output=True)
            if r.returncode == 0:
                return True
            time.sleep(0.5)
        self.stop()
        try:
            self.start()
            return True
        except RuntimeError:
            return False

    def createdb(self):
        self.dbs += 1
        name = "pt%d" % self.dbs
        subprocess.run([os.path.join(self.bin, "psql"), "-X", "-q", "-d", "postgres", "-c", "DROP DATABASE IF EXISTS %s WITH (FORCE)" % name], env=self.env(), capture_output=True)
        r = subprocess.run([os.path.join(self.bin, "createdb"), name], env=self.env(), capture_output=True, text=True)
        if r.returncode:
            raise RuntimeError("createdb: " + r.stderr[-300:])
        return name

    def log_size(self):
        try:
            return os.path.getsize(self.log)
        except OSError:
            return 0

    def log_since(self, offset):
        try:
            with open(self.log, errors="replace") as f:
                f.seek(offset)
                return f.read()
        except OSError:
            return ""

    def backends(self):
        pm = self.postmaster()
        if not pm:
            return []
        return [(pid, rss) for pid, (ppid, rss, cmd) in ps_table().items() if ppid == pm and "[local]" in cmd]


def run_watched(cmd, cluster, rss_mb, secs, env=None, stdin=None, poll=0.25):
    """Run `cmd` (a client of `cluster`), killing any client backend over
    `rss_mb` and the whole replay past `secs`. `(returncode, output, killed,
    elapsed, peak_rss_mb)`, killed None, `rss` or `time`."""
    t0 = time.time()
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL,
                         env=env or cluster.env(), start_new_session=True)
    chunks = []
    reader = threading.Thread(target=lambda: chunks.append(p.stdout.read()), daemon=True)
    reader.start()
    if stdin is not None:
        try:
            p.stdin.write(stdin.encode())
            p.stdin.close()
        except (BrokenPipeError, OSError):
            pass
    killed, peak = None, 0
    while p.poll() is None:
        for pid, rss in cluster.backends():
            peak = max(peak, rss // 1024)
            if rss // 1024 > rss_mb:
                killed = killed or "rss"
                _kill(pid, signal.SIGKILL, cluster)
        if time.time() - t0 > secs:
            killed = killed or "time"
            try:
                os.killpg(p.pid, signal.SIGKILL)
            except OSError:
                pass
            for pid, _ in cluster.backends():
                _kill(pid, signal.SIGTERM, cluster)
            time.sleep(3)
            for pid, _ in cluster.backends():
                _kill(pid, signal.SIGKILL, cluster)
            break
        time.sleep(poll)
    try:
        p.wait(timeout=10)
    except subprocess.TimeoutExpired:
        p.kill()
        p.wait()
    reader.join(timeout=5)
    out = b"".join(c for c in chunks if c).decode(errors="replace")
    return p.returncode, out, killed, time.time() - t0, peak


def _kill(pid, sig, cluster):
    try:
        os.kill(pid, sig)
        if sig == signal.SIGKILL:
            cluster.killed.append(pid)
    except OSError:
        pass


# ---------------------------------------------------------------- replays

class Replayer:
    """Replays findings on builds' clusters."""

    def __init__(self, args, runner):
        self.args, self.runner = args, runner
        self.scripts = {}          # finding id -> rebuilt script (memory, hangs)

    def methods(self, f):
        if f.kind == "failure":
            if not f.example:
                return []
            return (["psql"] if message_kind(f.message) == "sql-error" else []) + ["oracle"]
        if f.kind == "crash":
            return (["psql"] if f.scripts else []) + (["oracle"] if f.states else [])
        if f.kind in ("memory", "hangs"):
            return ["psql", "oracle"]
        if f.kind == "timeouts":
            return ["psql"]
        return []

    def replay(self, f, cluster, method):
        try:
            if method == "psql":
                return self._psql(f, cluster)
            return self._oracle(f, cluster)
        except (RuntimeError, OSError) as e:
            return Outcome(None, method, "replay failed: %s" % e)

    # psql: the finding's script(s).
    def _psql(self, f, cluster):
        a = self.args
        if f.kind == "crash":
            scripts = f.scripts
        elif f.kind in ("memory", "hangs"):
            s = self.rebuilt_script(f)
            if not s:
                return Outcome(None, "psql", "the input could not be rebuilt from the record")
            scripts = [s]
        else:
            scripts = [os.path.join(f.example, "script.sql")]
        # A resource finding's statements get a short timeout: a build that
        # honours it finishes, one that does not runs into the watchdog.
        resource = f.kind in ("memory", "hangs") or (f.kind == "crash" and f.signature in ("signal-9", "server-lost"))
        notes, fails, killed_any, timeout_seen, crash = [], False, None, False, []
        for s in scripts:
            text = read_text(s)
            if resource:
                text = re.sub(r"SET statement_timeout = '[^']*';", "SET statement_timeout = '%s';" % a.resource_timeout, text, count=1)
            if not cluster.ready():
                return Outcome(None, "psql", "the cluster does not come back")
            db = cluster.createdb()
            offset, before = cluster.log_size(), len(cluster.killed)
            fd, tmp = tempfile.mkstemp(prefix="ptscript", suffix=".sql", dir=a.tmp)
            with os.fdopen(fd, "w") as fh:
                fh.write(text)
            rc, out, killed, elapsed, peak = run_watched(
                [os.path.join(cluster.bin, "psql"), "-X", "-q", "-d", db, "-v", "ON_ERROR_STOP=0", "-v", "VERBOSITY=verbose", "-f", tmp],
                cluster, a.max_rss_mb, a.replay_secs)
            os.unlink(tmp)
            log = cluster.log_since(offset)
            crash = crash_lines(log, ignore_kill=len(cluster.killed) > before)
            errors = psql_errors(out)
            timeout_seen = timeout_seen or any(c == "57014" for c, _ in errors)
            killed_any = killed_any or killed
            if f.kind == "failure":
                exp = failing_line(f.message)
                hit = exp is not None and error_matches(exp, errors)
                fails = fails or hit or bool(crash)
                notes.append("%s: %s%s%s" % (os.path.basename(s), "error reproduced" if hit else "no matching error",
                                             ", crash" if crash else "", ", killed (%s)" % killed if killed else ""))
            else:
                fails = fails or bool(crash) or bool(killed)
                notes.append("%s: %.1fs, peak %d MB%s%s%s" % (os.path.basename(s), elapsed, peak, ", killed (%s)" % killed if killed else "",
                                                           ", crash: " + crash[0][:200] if crash else "", ", statement timeout" if timeout_seen else ""))
            cluster.ready()
        return Outcome(fails, "psql", "; ".join(notes), killed=killed_any, crash=crash[:3], timeout=timeout_seen)

    def rebuilt_script(self, f):
        if f.id in self.scripts:
            return self.scripts[f.id]
        path = None
        if self.runner:
            out = tempfile.mkdtemp(prefix="ptscript", dir=self.args.tmp)
            none = os.path.join(out, "none")
            os.makedirs(none)
            subprocess.run([self.runner, "replay", "--finding", f.record, "--out", out, "--pg-dir", none, "--pg-bin", none],
                           capture_output=True, timeout=120)
            s = os.path.join(out, "script.sql")
            path = s if os.path.exists(s) else None
        self.scripts[f.id] = path
        return path

    # oracle: `runner replay` against the cluster.
    def _oracle(self, f, cluster):
        a = self.args
        if not self.runner:
            return Outcome(None, "oracle", "no runner binary")
        target = f.example if f.kind in ("failure", "timeouts") else (f.states[0] if f.kind == "crash" and f.states else f.record)
        if not target:
            return Outcome(None, "oracle", "nothing to rebuild an input from")
        if not cluster.ready():
            return Outcome(None, "oracle", "the cluster does not come back")
        out = tempfile.mkdtemp(prefix="ptoracle", dir=a.tmp)
        offset, before = cluster.log_size(), len(cluster.killed)
        rc, text, killed, elapsed, peak = run_watched(
            [self.runner, "replay", "--finding", target, "--out", out, "--pg-dir", cluster.rdir, "--pg-bin", os.path.join(cluster.dir, "nobin")],
            cluster, a.max_rss_mb, a.replay_secs)
        crash = crash_lines(cluster.log_since(offset), ignore_kill=len(cluster.killed) > before)
        verdict = read_text(os.path.join(out, "verdict.txt")).strip()
        shutil.rmtree(out, ignore_errors=True)
        cluster.ready()
        cls, sig, vt = parse_oracle_verdict(verdict) if verdict else ("unknown", None, text.strip()[-300:])
        extra = {"killed": killed, "crash": crash[:3], "runner": cls}
        if f.kind == "failure":
            if cls == "fail":
                if sig != f.signature:
                    extra["other_signature"] = sig
                return Outcome(True, "oracle", vt, **extra)
            if cls in ("known", "lost") or crash or killed:
                return Outcome(True, "oracle", vt or "killed (%s)" % killed, **extra)
            if cls in ("pass", "timeout"):
                return Outcome(False, "oracle", vt, **extra)
            return Outcome(None, "oracle", "the runner gave no verdict: " + vt, **extra)
        # crash, memory, hangs: a crash or a kill reproduces it; any verdict in time does not.
        if crash or killed or cls == "lost":
            return Outcome(True, "oracle", "%s%s%s" % (vt[:200], ", killed (%s)" % killed if killed else "", ", crash: " + crash[0][:200] if crash else ""), **extra)
        if cls == "unknown":
            return Outcome(None, "oracle", "the runner gave no verdict: " + vt, **extra)
        return Outcome(False, "oracle", "%s (%.1fs, peak %d MB)" % (vt[:200], elapsed, peak), **extra)


# ---------------------------------------------------------------- builds

def load_builds(args, registry):
    """`(builds, commit, commit_date)`: builds `{name: bindir}`, master
    first, then each open bug's patched build whose status is ok."""
    builds, commit, date = {}, None, None
    d = args.builds
    status = read_json(os.path.join(d, "status.json")) if d else None
    if status:
        commit, date = status.get("commit"), status.get("commit_date")
        names = ["master"] + [b["id"] for b in open_bugs(registry)]
        for n in names:
            st = status.get("builds", {}).get(n, {}).get("status")
            bindir = os.path.join(d, n, "inst", "bin")
            if st == "ok" and os.path.exists(os.path.join(bindir, "postgres")):
                builds[n] = bindir
    if args.master_bin:
        builds = dict([("master", args.master_bin)] + [(k, v) for k, v in builds.items() if k != "master"])
        commit = git_commit_of_bin(args.master_bin) or commit
    for spec in args.build or []:
        n, _, b = spec.partition("=")
        builds[n] = b
    if "master" in builds:
        builds = dict([("master", builds["master"])] + [(k, v) for k, v in builds.items() if k != "master"])
    return builds, commit, date


def git_commit_of_bin(bindir):
    """The commit of the checkout `<checkout>/inst/bin` was installed from
    (the runner's convention), or None."""
    r = subprocess.run(["git", "-C", os.path.join(bindir, "..", ".."), "rev-parse", "HEAD"], capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 and os.path.exists(os.path.join(bindir, "..", "..", "configure")) else None


def is_ancestor(src, older, newer):
    if not src or not older or not newer:
        return None
    r = subprocess.run(["git", "-C", src, "merge-base", "--is-ancestor", older, newer], capture_output=True)
    return {0: True, 1: False}.get(r.returncode)


def runner_built_version(runner):
    if not runner:
        return None
    r = subprocess.run([runner, "version"], capture_output=True, text=True)
    m = re.search(r"built from (\S+)", r.stdout)
    return m.group(1) if m else None


# ---------------------------------------------------------------- passes

def run_dir_of(path):
    p = os.path.abspath(path)
    while p != os.path.dirname(p):
        if os.path.exists(os.path.join(p, "run.json")):
            return p
        p = os.path.dirname(p)
    return None


def targets(paths, root):
    """`[(findings_dir, run_dir or None)]`."""
    out = []
    for p in paths or [os.path.join(root, "current")]:
        p = os.path.realpath(p)
        if os.path.isdir(os.path.join(p, "findings")):
            out.append((os.path.join(p, "findings"), p))
        elif os.path.isdir(p):
            out.append((p, run_dir_of(p)))
    return out


def append_line(path, obj):
    with open(path, "a") as f:
        f.write(json.dumps(obj, sort_keys=True) + "\n")


def write_atomic(path, text):
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        f.write(text)
    os.replace(tmp, path)


def deploy_log(root, line):
    if root:
        os.makedirs(root, exist_ok=True)
        with open(os.path.join(root, "deploy.log"), "a") as f:
            f.write("%s pretriage.py: %s\n" % (time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), line))


def summary_line(run_name, v):
    reason = v.get("reason", "")
    return "%s %s %s%s" % (run_name, v["finding"], label(v), " (%s)" % reason[:160] if reason else "")


def assert_only_in(message, target_src, master_src):
    """Whether the assertion a crash message names (`TRAP: failed
    Assert("expr"), File: "x.c"`) is in x.c under `target_src` but not
    under `master_src` (source trees of two builds)."""
    m = re.search(r'TRAP: failed Assert\("(.*?)"\), File: "([^"]+)"', message or "")
    if not m or not target_src or not master_src:
        return False
    expr, name = m.group(1), os.path.basename(m.group(2))

    def has(src):
        for d, _, files in os.walk(os.path.join(src, "src")):
            if name in files and expr in read_text(os.path.join(d, name)):
                return True
        return False
    return has(target_src) and not has(master_src)


def judge(f, replayer, clusters, registry, ctx, built, target_cluster=None):
    """Replay one finding as the module docs say and decide its verdict."""
    outcomes = {}
    if f.kind in HARNESS_KINDS or (f.kind == "failure" and message_kind(f.message) == "harness"):
        return decide(f, outcomes, registry, ctx, built)
    master = clusters.get("master")
    if master is None:
        return decide(f, {"master": Outcome(None, "-", "no master build")}, registry, ctx, built)
    methods = replayer.methods(f)
    if not methods:
        return decide(f, {"master": Outcome(None, "-", "no script, state or example to replay")}, registry, ctx, built)
    chosen, tried = None, []
    for m in methods:
        o = replayer.replay(f, master, m)
        tried.append(o)
        if o.fails:
            chosen = (m, o)
            break
    if not chosen:
        # Master does not reproduce it: an outcome that ran (passes) stands,
        # else the first (could not run).
        outcomes["master"] = next((o for o in tried if o.fails is False), tried[0])
        target = None
        if outcomes["master"].fails is False and target_cluster is not None:
            for m in methods:
                target = replayer.replay(f, target_cluster, m)
                if target.fails:
                    break
        return decide(f, outcomes, registry, ctx, built, target=target, tried=tried)
    method, outcomes["master"] = chosen
    others = [n for n in clusters if n != "master"]
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, min(len(others), replayer.args.jobs))) as ex:
        futs = {n: ex.submit(replayer.replay, f, clusters[n], method) for n in others}
        for n in others:
            outcomes[n] = futs[n].result()
    return decide(f, outcomes, registry, ctx, built, tried=tried)


def pretriage(args, registry):
    builds, master_commit, _ = load_builds(args, registry)
    if "master" not in builds:
        print("no master build (--builds with a status.json, or --master-bin)", file=sys.stderr)
        deploy_log(args.root, "no master build: run build-patched.sh")
        return 1
    runner = args.runner if args.runner and os.path.exists(args.runner) else None
    runner_version = runner_built_version(runner)
    todo = []
    for findings, run in targets(args.paths, args.root):
        fs = discover(findings, args.force)
        if fs:
            todo.append((findings, run, fs))
    if not todo:
        return 0
    os.makedirs(args.tmp, exist_ok=True)
    root = tempfile.mkdtemp(prefix="pgpt", dir="/tmp")    # short: unix socket paths
    clusters = {}
    targets_c = {}
    replayer = Replayer(args, runner)
    try:
        for k, (n, b) in enumerate(builds.items()):
            c = Cluster(n, b, root, args.port_base + k)
            c.start()
            clusters[n] = c
        deadline = time.time() + args.budget
        for findings, run, fs in todo:
            header = read_json(os.path.join(run, "run.json")) if run else None
            run_commit = ((header or {}).get("target") or {}).get("commit") or (header or {}).get("target_id")
            ctx = Context(run_commit, master_commit, is_ancestor(args.pg_src, run_commit, master_commit),
                          bool(header and runner_version and header.get("version_id") not in (None, runner_version)),
                          bool(run_commit and args.pg_builds and os.path.exists(os.path.join(args.pg_builds, run_commit[:12], "fixes.json"))))
            tc = None
            if run_commit and master_commit and run_commit[:12] != master_commit[:12] and args.pg_builds:
                tb = os.path.join(args.pg_builds, run_commit[:12], "inst", "bin")
                if os.path.exists(os.path.join(tb, "postgres")):
                    if run_commit[:12] not in targets_c:
                        c = Cluster("target-" + run_commit[:12], tb, root, args.port_base + len(clusters) + len(targets_c))
                        c.start()
                        targets_c[run_commit[:12]] = c
                    tc = targets_c[run_commit[:12]]
            run_name = os.path.basename(run) if run else findings
            out_dir = run or findings
            for f in fs:
                if time.time() > deadline:
                    deploy_log(args.root, "%s: budget of %ds spent, the rest next pass" % (run_name, args.budget))
                    return 0
                ctx.assert_fixes_only = bool(ctx.run_fixes and f.kind == "crash" and assert_only_in(
                    f.message, os.path.join(args.pg_builds, (run_commit or "")[:12]), os.path.dirname(os.path.dirname(builds["master"]))))
                v = judge(f, replayer, clusters, registry, ctx, built=[n for n in builds if n != "master"], target_cluster=tc)
                v.update(ts=int(time.time()), run=(header or {}).get("run_id"), signature=f.signature,
                         builds={n: c.bin for n, c in clusters.items()}, master_commit=master_commit)
                write_atomic(f.verdict_path, json.dumps(v, indent=2, sort_keys=True) + "\n")
                append_line(os.path.join(out_dir, "pretriage.jsonl"), v)
                deploy_log(args.root, summary_line(run_name, v))
                print(summary_line(run_name, v))
    finally:
        for c in list(clusters.values()) + list(targets_c.values()):
            c.stop()
        shutil.rmtree(root, ignore_errors=True)
    return 0


# ---------------------------------------------------------------- known-bug checks

def run_check(cluster, bug):
    """`(output, killed, elapsed, peak_mb)` of a bug's check in a fresh
    database of `cluster`."""
    lim = bug.get("limits", {})
    if not cluster.ready():
        raise RuntimeError("the cluster does not come back")
    db = cluster.createdb()
    rc, out, killed, elapsed, peak = run_watched(
        [os.path.join(cluster.bin, "psql"), "-X", "-q", "-At", "-d", db, "-v", "ON_ERROR_STOP=0", "-v", "VERBOSITY=sqlstate", "-f", "-"],
        cluster, lim.get("rss_mb", 4096), lim.get("seconds", 300), stdin=bug["check"])
    cluster.ready()
    return out, killed, elapsed, peak


def check_ok(bug, result):
    out, killed, elapsed, _ = result
    lim = bug.get("limits", {})
    return out == bug.get("expected") and not killed and elapsed <= lim.get("seconds", 1e9)


def check_status(bug, master, patched):
    """open, fixed-on-master, fix-broken, open-no-build or no-expected, from the
    check results (`(output, killed, elapsed, peak)` or None)."""
    if bug.get("expected") is None:
        return "no-expected"
    if master is not None and check_ok(bug, master):
        return "fixed-on-master"
    if patched is None:
        return "open-no-build"
    if not check_ok(bug, patched):
        return "fix-broken"
    return "open"


def with_clusters(args, names_bins, fn):
    root = tempfile.mkdtemp(prefix="pgpt", dir="/tmp")
    clusters = {}
    try:
        for k, (n, b) in enumerate(names_bins):
            c = Cluster(n, b, root, args.port_base + k)
            c.start()
            clusters[n] = c
        return fn(clusters)
    finally:
        for c in clusters.values():
            c.stop()
        shutil.rmtree(root, ignore_errors=True)


def check_bugs(args, registry):
    builds, commit, date = load_builds(args, registry)
    if "master" not in builds:
        print("no master build", file=sys.stderr)
        return 1

    def go(clusters):
        report = {"ts": int(time.time()), "master_commit": commit, "master_commit_date": date, "bugs": {}, "retired": []}
        for bug in registry["bugs"]:
            if bug.get("status") != "open":
                report["retired"].append(bug["id"])
                continue
            m = run_check(clusters["master"], bug)
            p = run_check(clusters[bug["id"]], bug) if bug["id"] in clusters else None
            st = check_status(bug, m, p)
            fmt = lambda r: None if r is None else {"output": r[0][-2000:], "killed": r[1], "seconds": round(r[2], 2), "peak_rss_mb": r[3], "ok": check_ok(bug, r)}
            report["bugs"][bug["id"]] = {"status": st, "master": fmt(m), "patched": fmt(p)}
            line = "known bug %s: %s on master %s" % (bug["id"], st, (commit or "?")[:12])
            deploy_log(args.root, line)
            print(line)
        return report

    report = with_clusters(args, builds.items(), go)
    if args.root:
        write_atomic(os.path.join(args.root, "known-bugs-status.json"), json.dumps(report, indent=2, sort_keys=True) + "\n")
    return 0


def print_expected(args, registry, names):
    builds, _, _ = load_builds(args, registry)
    bugs = [b for b in open_bugs(registry) if not names or b["id"] in names]
    todo = [(b["id"], builds[b["id"]]) for b in bugs if b["id"] in builds]
    for b in bugs:
        if b["id"] not in builds:
            print("== %s: no patched build" % b["id"])

    def go(clusters):
        for b in bugs:
            if b["id"] in clusters:
                out, killed, elapsed, peak = run_check(clusters[b["id"]], b)
                print("== %s (%.2fs, peak %d MB%s)" % (b["id"], elapsed, peak, ", killed: %s" % killed if killed else ""))
                print(json.dumps(out))
    if todo:
        with_clusters(args, todo, go)
    return 0


# ---------------------------------------------------------------- main

def main(argv=None):
    ap = argparse.ArgumentParser(description="Pre-triage of the PostgreSQL fuzz runner's findings (module docs).")
    ap.add_argument("paths", nargs="*", help="run or findings directories (default RUN_ROOT/current)")
    ap.add_argument("--root", default=os.environ.get("RUN_ROOT", os.path.expanduser("~/fuzz-runs")), help="RUN_ROOT")
    ap.add_argument("--registry", default=os.path.join(HERE, "known-bugs.json"))
    ap.add_argument("--builds", default=None, help="build-patched.sh's OUT/current (default PG_BUILDS/patched/current)")
    ap.add_argument("--master-bin", default=None, help="master's bin directory instead of the builds' master")
    ap.add_argument("--build", action="append", help="NAME=BINDIR: add or replace a build")
    ap.add_argument("--pg-builds", default=os.environ.get("PG_BUILDS", os.path.expanduser("~/pg-builds")), help="build-pg.sh's builds: the run's own commit's, for fixed-on-master")
    ap.add_argument("--pg-src", default=os.environ.get("PG_SRC"), help="the PostgreSQL clone, to order commits")
    ap.add_argument("--runner", default=os.path.join(os.path.dirname(HERE), "target", "release", "runner"))
    ap.add_argument("--port-base", type=int, default=int(os.environ.get("PRETRIAGE_PORT_BASE", "5641")))
    ap.add_argument("--max-rss-mb", type=int, default=4096)
    ap.add_argument("--replay-secs", type=int, default=180, help="wall-clock limit of one replay")
    ap.add_argument("--resource-timeout", default="2s", help="statement_timeout for memory, hang and crash scripts")
    ap.add_argument("--budget", type=int, default=1500, help="seconds per pass")
    ap.add_argument("--jobs", type=int, default=4, help="patched builds replayed at once")
    ap.add_argument("--tmp", default=None, help="scratch directory (default RUN_ROOT/.pretriage-tmp)")
    ap.add_argument("--force", action="store_true", help="judge findings that have a verdict again")
    ap.add_argument("--check-bugs", action="store_true")
    ap.add_argument("--expected", action="store_true", help="print the named bugs' (default all) check output on their patched builds")
    args = ap.parse_args(argv)
    if args.builds is None:
        args.builds = os.path.join(os.environ.get("PG_PATCHED", os.path.join(args.pg_builds, "patched")), "current")
    args.tmp = args.tmp or os.path.join(args.root, ".pretriage-tmp")
    registry = load_registry(args.registry)
    if args.check_bugs:
        return check_bugs(args, registry)
    if args.expected:
        return print_expected(args, registry, args.paths)
    try:
        return pretriage(args, registry)
    finally:
        shutil.rmtree(args.tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
