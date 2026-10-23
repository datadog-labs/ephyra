# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Unit tests of pretriage.py: fake findings, fake replay outcomes and fake
build outputs; no PostgreSQL needed. `python3 -m unittest` in deploy/."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import types
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pretriage as pt  # noqa: E402

REAL_REPLAYER = pt.Replayer
REGISTRY = pt.load_registry(os.path.join(pt.HERE, "known-bugs.json"))
# The repository, which known-bugs.json's paths are relative to.
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(pt.HERE))))

PSQL_MASTER = """psql:/tmp/x.sql:14: ERROR:  22008: cannot subtract infinite dates
LOCATION:  date_mi, date.c:556
"""
PSQL_NODE = "psql:/tmp/x.sql:9: ERROR:  XX000: unrecognized node type: 7\nLOCATION:  create_plan_recurse, createplan.c:544\n"
PSQL_LOST = "psql:/tmp/x.sql:3: server closed the connection unexpectedly\n\tThis probably means the server terminated abnormally\n"


def finding(kind="failure", message="", signature=None, ident="x"):
    return pt.Finding(kind, ident, "/nonexistent", "/nonexistent/verdict.json", message=message, signature=signature)


class Parsing(unittest.TestCase):
    def test_fold_masks_digits(self):
        self.assertEqual(pt.fold("unrecognized node type: 10"), pt.fold("unrecognized  node type: 7"))

    def test_failing_line(self):
        self.assertEqual(pt.failing_line("statement 2: INSERT INTO t VALUES ('-infinity'): cannot subtract infinite dates (22008)"),
                         ("22008", "statement 2: INSERT INTO t VALUES ('-infinity'): cannot subtract infinite dates"))
        self.assertEqual(pt.failing_line("the original query fails: SELECT 1\n  unrecognized node type: 10 (XX000)")[0], "XX000")
        self.assertIsNone(pt.failing_line("tlp-where: results differ\n  original: SELECT 1"))

    def test_message_kind(self):
        self.assertEqual(pt.message_kind("statement 3: UPDATE t SET c = 1: boom (XX000)"), "sql-error")
        self.assertEqual(pt.message_kind("the original query fails: SELECT 1\n  x (42883)"), "sql-error")
        self.assertEqual(pt.message_kind("norec: results differ\n  original: SELECT 1"), "oracle")
        self.assertEqual(pt.message_kind("statement 3: SELECT 1: the model has no rows for it"), "oracle")
        self.assertEqual(pt.message_kind("model: index check fails\n  SELECT bt_index_check('i'): bad (XX002)"), "oracle")
        self.assertEqual(pt.message_kind("panic: index out of bounds"), "harness")

    def test_psql_errors_and_matching(self):
        errs = pt.psql_errors(PSQL_MASTER)
        self.assertEqual(errs, [("22008", "cannot subtract infinite dates")])
        exp = pt.failing_line("statement 2: INSERT INTO t VALUES ('-infinity'): cannot subtract infinite dates (22008)")
        self.assertTrue(pt.error_matches(exp, errs))
        self.assertFalse(pt.error_matches(exp, [("22012", "division by zero")]))
        self.assertFalse(pt.error_matches(exp, [("22008", "date out of range")]))
        # A node number that differs between builds still matches.
        node = pt.failing_line("statement 4: SELECT 0::int8 IN (SELECT 1): unrecognized node type: 10 (XX000)")
        self.assertTrue(pt.error_matches(node, pt.psql_errors(PSQL_NODE)))
        self.assertTrue(pt.psql_lost(PSQL_LOST))
        self.assertFalse(pt.psql_lost(PSQL_MASTER))

    def test_crash_lines(self):
        log = ("LOG:  client backend (PID 7) was terminated by signal 9: Killed: 9\n"
               "LOG:  all server processes terminated; reinitializing\n"
               "LOG:  client backend (PID 8) exited with exit code 1\n"
               "TRAP: failed Assert(\"x\"), File: \"a.c\", Line: 3, PID: 9\n")
        self.assertEqual(len(pt.crash_lines(log)), 3)
        self.assertEqual(pt.crash_lines(log, ignore_kill=True), ['TRAP: failed Assert("x"), File: "a.c", Line: 3, PID: 9'])
        self.assertEqual(pt.crash_lines("LOG:  server process (PID 1) was terminated by signal 11: Segmentation fault\n", ignore_kill=True)[0][:5], "LOG: ")

    def test_oracle_verdicts(self):
        self.assertEqual(pt.parse_oracle_verdict("passed: 2 queries, 14 comparisons")[0], "pass")
        self.assertEqual(pt.parse_oracle_verdict("failed (tlp-tlp-where__results_differ): tlp-where: results differ")[:2], ("fail", "tlp-tlp-where__results_differ"))
        self.assertEqual(pt.parse_oracle_verdict("a known bug: unrecognized node type: 7 (XX000)")[0], "known")
        self.assertEqual(pt.parse_oracle_verdict("a statement timeout: canceling statement (57014)")[0], "timeout")
        self.assertEqual(pt.parse_oracle_verdict("the session was lost (a crash?): connection closed")[0], "lost")
        self.assertEqual(pt.parse_oracle_verdict("garbage")[0], "unknown")


class Registry(unittest.TestCase):
    def test_entries_are_complete(self):
        ids = set()
        for b in REGISTRY["bugs"]:
            ids.add(b["id"])
            for k in ("folder", "summary", "status", "fix", "signatures", "reproduce", "check", "excluded_by"):
                self.assertIn(k, b, (b["id"], k))
            self.assertTrue(os.path.isdir(os.path.join(ROOT, b["folder"])), b["folder"])
            self.assertIn(b["fix"]["kind"], ("core", "contrib"))
            # Each fix is available locally or says how to get it.
            local = any("path" in s and os.path.isfile(os.path.join(ROOT, s["path"])) for s in b["fix"]["sources"])
            self.assertTrue(local or b["fix"].get("obtain"), b["id"])
            if not os.path.isfile(os.path.join(ROOT, b["fix"]["sources"][0].get("path", "-"))):
                self.assertTrue(b["fix"].get("optional"), "%s: first source not saved, so optional" % b["id"])
            self.assertIn(b["reproduce"], ("single-session", "needs-concurrency"))
            self.assertIsNotNone(b["expected"], b["id"])
        self.assertEqual(ids, {"unrecognized-node-10", "toast-missing-chunk-vacuum-full", "btree-gist-infinite-date", "huge-expr-timeout",
                               "hashsetop-out-of-memory", "cluster-subxact-row-loss", "window-exclude-ties-mark",
                               "parallel-material-dup-toc", "viewdef-negated-sort-constant", "viewdef-sublink-paren",
                               "toast-v7-recently-dead-assert",
                               "heap-update-locked-only-assert"})

    def test_matching(self):
        f = finding(message="statement 4: SELECT 1: unrecognized node type: 10 (XX000)", signature="model-statement-SELECT-XX000")
        self.assertIn(("unrecognized-node-10", "error"), pt.match_bugs(f, REGISTRY))
        f = finding(message="statement 18 of the case: VACUUM (FULL, ANALYZE) t3: missing chunk number 0 for toast value 6824744 in pg_toast_6824702 (XX001)",
                    signature="plans-statement-VACUUM-XX001")
        self.assertEqual(pt.match_bugs(f, REGISTRY), [("toast-missing-chunk-vacuum-full", "error")])
        f = finding("memory", message="backend over 4096 MB")
        self.assertEqual(pt.match_bugs(f, REGISTRY), [("huge-expr-timeout", "kind"), ("hashsetop-out-of-memory", "kind")])
        f = finding(message="the original query fails: SELECT 1\n  out of memory (53200)", signature="plans-original-fails-__SELECT-53200")
        self.assertEqual(pt.match_bugs(f, REGISTRY), [("hashsetop-out-of-memory", "error")])
        f = finding(message="the original query fails: SELECT 1\n  cannot fetch row before WindowObject's mark position (XX000)",
                    signature="plans-original-fails-SELECT-XX000")
        self.assertIn(("window-exclude-ties-mark", "error"), pt.match_bugs(f, REGISTRY))
        f = finding(message="tlp-where: results differ", signature="tlp-tlp-where__results_differ")
        self.assertEqual(pt.match_bugs(f, REGISTRY), [])


GIST = "statement 2: INSERT INTO t VALUES ('-infinity'): cannot subtract infinite dates (22008)"
TOAST = "statement 18 of the case: VACUUM (FULL, ANALYZE) t3: missing chunk number 0 for toast value 1 in pg_toast_2 (XX001)"
NODE = "the original query fails: SELECT 1\n  unrecognized node type: 10 (XX000)"
F, P, X = True, False, None
SAME = pt.Context("158b6d5109fa" + "0" * 28, "158b6d5109fa" + "0" * 28)


def o(fails, method="psql", **kw):
    return pt.Outcome(fails, method, "fake", **kw)


class Decide(unittest.TestCase):
    def test_assert_only_in(self):
        with tempfile.TemporaryDirectory() as d:
            for name, body in (("fix", 'Assert(recently_dead);\n'), ("master", '/* nothing */\n')):
                os.makedirs(os.path.join(d, name, "src", "backend"))
                open(os.path.join(d, name, "src", "backend", "heapam_handler.c"), "w").write(body)
            msg = 'TRAP: failed Assert("recently_dead"), File: "heapam_handler.c", Line: 993, PID: 1'
            self.assertTrue(pt.assert_only_in(msg, os.path.join(d, "fix"), os.path.join(d, "master")))
            self.assertFalse(pt.assert_only_in(msg, os.path.join(d, "master"), os.path.join(d, "fix")))
            self.assertFalse(pt.assert_only_in("no trap here", os.path.join(d, "fix"), os.path.join(d, "master")))
        ctx = pt.Context("aaaaaaaaaaaa" + "0" * 28, "158b6d5109fa" + "0" * 28, False, run_fixes=True)
        ctx.assert_fixes_only = True
        v = pt.decide(finding("crash", message="TRAP"), {"master": o(P)}, REGISTRY, ctx)
        self.assertEqual((v["verdict"], v["basis"]), ("fixes-only", "source"))

    def test_fixes_only(self):
        ctx = pt.Context("aaaaaaaaaaaa" + "0" * 28, "158b6d5109fa" + "0" * 28, False, run_fixes=True)
        f = finding(message="tlp-where: results differ", signature="tlp-tlp-where__results_differ")
        v = pt.decide(f, {"master": o(P)}, REGISTRY, ctx, target=o(F))
        self.assertEqual(v["verdict"], "fixes-only")
        v = pt.decide(f, {"master": o(P)}, REGISTRY, ctx, target=o(P))
        self.assertEqual(v["verdict"], "not-reproduced")

    def test_known_by_replay(self):
        v = pt.decide(finding(message=GIST, signature="model-statement-INSERT-22008"),
                      {"master": o(F), "btree-gist-infinite-date": o(P), "huge-expr-timeout": o(F)}, REGISTRY, SAME,
                      built=["btree-gist-infinite-date", "huge-expr-timeout"])
        self.assertEqual(v["verdict"], "known:btree-gist-infinite-date")
        self.assertEqual(v["bugs"], ["btree-gist-infinite-date"])
        self.assertEqual(v["basis"], "replay")

    def test_new_when_every_build_fails(self):
        v = pt.decide(finding(message="norec: results differ", signature="norec-norec__results_differ"),
                      {"master": o(F, "oracle"), "btree-gist-infinite-date": o(F, "oracle")}, REGISTRY, SAME, built=["btree-gist-infinite-date"])
        self.assertEqual(v["verdict"], "new")

    def test_known_by_signature_without_a_build(self):
        v = pt.decide(finding(message=NODE, signature="norec-original-fails-SELECT-XX000"),
                      {"master": o(F), "btree-gist-infinite-date": o(F)}, REGISTRY, SAME, built=["btree-gist-infinite-date"])
        self.assertEqual(v["verdict"], "known:unrecognized-node-10")
        self.assertEqual(v["basis"], "signature")
        # With its build there and failing too, it is new.
        v = pt.decide(finding(message=NODE, signature="norec-original-fails-SELECT-XX000"),
                      {"master": o(F), "unrecognized-node-10": o(F)}, REGISTRY, SAME, built=["unrecognized-node-10"])
        self.assertEqual(v["verdict"], "new")

    def test_not_reproduced_needs_concurrency(self):
        v = pt.decide(finding(message=TOAST, signature="plans-statement-VACUUM-XX001"), {"master": o(P)}, REGISTRY, SAME)
        self.assertEqual(v["verdict"], "not-reproduced")
        self.assertEqual(pt.label(v), "not-reproduced:needs-concurrency:toast-missing-chunk-vacuum-full")
        self.assertIn("needs concurrency", v["reason"])

    def test_not_reproduced_plain_and_drift(self):
        v = pt.decide(finding(message="tlp-where: results differ", signature="tlp-x"), {"master": o(P, "oracle")}, REGISTRY, SAME)
        self.assertEqual(pt.label(v), "not-reproduced")
        drift = pt.Context(SAME.run_commit, SAME.master_commit, None, True)
        v = pt.decide(finding(message="tlp-where: results differ", signature="tlp-x"), {"master": o(P, "oracle")}, REGISTRY, drift)
        self.assertTrue(v["generator_drift"])
        self.assertIn("generator", v["reason"])

    def test_fixed_on_master(self):
        old = pt.Context("aaaaaaaaaaaa" + "0" * 28, "bbbbbbbbbbbb" + "0" * 28, True)
        f = finding(message=GIST, signature="model-statement-INSERT-22008")
        v = pt.decide(f, {"master": o(P)}, REGISTRY, old, target=o(F))
        self.assertEqual((v["verdict"], v["confirmed"]), ("fixed-on-master", True))
        v = pt.decide(f, {"master": o(P)}, REGISTRY, old)
        self.assertEqual((v["verdict"], v["confirmed"]), ("fixed-on-master", False))
        v = pt.decide(f, {"master": o(P)}, REGISTRY, old, target=o(P))
        self.assertEqual(v["verdict"], "not-reproduced")
        # A run on a newer commit than master's (a stale master build) is not fixed on master.
        newer = pt.Context(old.run_commit, old.master_commit, False)
        self.assertEqual(pt.decide(f, {"master": o(P)}, REGISTRY, newer)["verdict"], "not-reproduced")

    def test_slow_query_harness_manual(self):
        v = pt.decide(finding("timeouts", message="canceling statement due to statement timeout (57014)"), {"master": o(P, timeout=True)}, REGISTRY, SAME)
        self.assertEqual(v["verdict"], "slow-query")
        self.assertEqual(pt.decide(finding("generator-panics"), {}, REGISTRY, SAME)["verdict"], "harness")
        self.assertEqual(pt.decide(finding(message="panic: boom"), {}, REGISTRY, SAME)["verdict"], "harness")
        self.assertEqual(pt.decide(finding(message=GIST), {"master": o(X)}, REGISTRY, SAME)["verdict"], "manual")


def make_run(root):
    """A fake run: run.json and a findings tree of each kind."""
    run = os.path.join(root, "20261014-0000-postgresql-158b6d5109fa")
    fs = os.path.join(run, "findings")
    os.makedirs(fs)
    with open(os.path.join(run, "run.json"), "w") as f:
        json.dump({"run_id": "r1", "version_id": "v1", "target_id": "158b6d5109fa", "target": {"commit": "158b6d5109fa" + "0" * 28}}, f)
    os.symlink(run, os.path.join(root, "current"))

    def write(path, text):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(text)
    for sig, msg in (("model-statement-INSERT-22008", GIST), ("plans-statement-VACUUM-XX001", TOAST), ("norec-norec__results_differ", "norec: results differ\n  original: SELECT 1")):
        for ex in ("w1-5", "w2-9"):
            write(os.path.join(fs, sig, ex, "message.txt"), msg)
            write(os.path.join(fs, sig, ex, "script.sql"), "SELECT 1;\n")
            write(os.path.join(fs, sig, ex, "state.txt"), "seed=1\nsize=2\noracle=model\n")
        write(os.path.join(fs, sig, "finding.json"), "{}")
        write(os.path.join(fs, sig, "count"), "2")
    crash = os.path.join(fs, "crashes", "signal-9")
    for t, culprit in (("1700000000", 1), ("1700000100", 3)):
        write(os.path.join(crash, t, "crash.txt"), "server crash, signature signal-9\nculprit worker(s), whose backend the log names: [%d]\n" % culprit)
        for w in (1, 2, 3):
            write(os.path.join(crash, t, "w%d.sql" % w), "SELECT %d;\n" % w)
            write(os.path.join(crash, t, "w%d.state.txt" % w), "seed=%d\nsize=1\n" % w)
    write(os.path.join(fs, "memory", "w2-1700000200.txt"), "backend over 4096 MB\n\nstate (the input in flight):\nseed=3\nsize=4\n")
    write(os.path.join(fs, "timeouts", "w1-77", "message.txt"), "canceling statement due to statement timeout (57014)")
    write(os.path.join(fs, "timeouts", "w1-77", "script.sql"), "SELECT 1;\n")
    write(os.path.join(fs, "timeouts", "w1-77", "state.txt"), "seed=1\nsize=1\n")
    write(os.path.join(fs, "generator-panics", "w1-1700000300.txt"), "panic\n")
    return run


class Discover(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.run = make_run(self.root)

    def tearDown(self):
        shutil.rmtree(self.root)

    def test_every_kind_once(self):
        fs = {f.id: f for f in pt.discover(os.path.join(self.run, "findings"))}
        self.assertEqual(set(fs), {"model-statement-INSERT-22008", "plans-statement-VACUUM-XX001", "norec-norec__results_differ",
                                   "crashes/signal-9", "memory/w2-1700000200", "timeouts/w1-77", "generator-panics/w1-1700000300"})
        g = fs["model-statement-INSERT-22008"]
        self.assertEqual((g.kind, os.path.basename(g.example), g.message), ("failure", "w1-5", GIST))
        c = fs["crashes/signal-9"]
        # The newest record, its culprit only.
        self.assertEqual([os.path.relpath(s, c.path) for s in c.scripts], ["1700000100/w3.sql"])
        self.assertEqual([os.path.basename(s) for s in c.states], ["w3.state.txt"])
        self.assertTrue(fs["memory/w2-1700000200"].verdict_path.endswith("memory/w2-1700000200.verdict.json"))
        self.assertTrue(fs["timeouts/w1-77"].verdict_path.endswith("timeouts/w1-77/verdict.json"))

    def test_judged_findings_are_skipped_unless_forced(self):
        fs = os.path.join(self.run, "findings")
        for f in pt.discover(fs):
            with open(f.verdict_path, "w") as h:
                h.write("{}")
        self.assertEqual(pt.discover(fs), [])
        self.assertEqual(len(pt.discover(fs, force=True)), 7)
        # Verdict files are not records themselves.
        self.assertEqual(len([f for f in pt.discover(fs, force=True) if f.kind == "memory"]), 1)


class FakeCluster:
    def __init__(self, name, bindir, root, port):
        self.name, self.bin, self.port = name, bindir, port
        self.killed = []

    def start(self):
        pass

    def stop(self):
        pass


class FakeReplayer:
    """Outcomes from a table: {(finding id, build, method): fails}; missing
    means passes. Records every call."""

    table = {}
    calls = []

    def __init__(self, args, runner):
        self.args = args

    def methods(self, f):
        return REAL_REPLAYER.methods(self, f)

    def replay(self, f, cluster, method):
        FakeReplayer.calls.append((f.id, cluster.name, method))
        fails = FakeReplayer.table.get((f.id, cluster.name, method), False)
        return pt.Outcome(fails, method, "fake %s on %s" % (method, cluster.name), timeout=(f.kind == "timeouts" and cluster.name == "master"))


class EndToEnd(unittest.TestCase):
    """pretriage() over a fake run with fake builds, clusters and replays."""

    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.run = make_run(self.root)
        self.builds = os.path.join(self.root, "patched", "158b6d5109fa")
        status = {"commit": "158b6d5109fa" + "0" * 28, "commit_date": "2026-10-13T14:31:45+03:00", "builds": {}}
        for n, st in (("master", "ok"), ("btree-gist-infinite-date", "ok"), ("huge-expr-timeout", "build_failed"), ("unrecognized-node-10", "skipped")):
            b = os.path.join(self.builds, n, "inst", "bin")
            os.makedirs(b)
            open(os.path.join(b, "postgres"), "w").close()
            status["builds"][n] = {"status": st}
        with open(os.path.join(self.builds, "status.json"), "w") as f:
            json.dump(status, f)
        self.saved = pt.Cluster, pt.Replayer, pt.runner_built_version
        pt.Cluster, pt.Replayer = FakeCluster, FakeReplayer
        pt.runner_built_version = lambda runner: "v1"
        FakeReplayer.calls = []
        FakeReplayer.table = {
            ("model-statement-INSERT-22008", "master", "psql"): True,
            ("norec-norec__results_differ", "master", "oracle"): True,
            ("norec-norec__results_differ", "btree-gist-infinite-date", "oracle"): True,
            ("crashes/signal-9", "master", "psql"): True,
            ("crashes/signal-9", "btree-gist-infinite-date", "psql"): True,
        }

    def tearDown(self):
        pt.Cluster, pt.Replayer, pt.runner_built_version = self.saved
        shutil.rmtree(self.root)

    def args(self, *extra):
        return ["--root", self.root, "--builds", self.builds, "--runner", "/bin/true", "--pg-builds", os.path.join(self.root, "none")] + list(extra)

    def test_verdicts_written(self):
        self.assertEqual(pt.main(self.args()), 0)
        fs = os.path.join(self.run, "findings")
        verdict = lambda p: json.load(open(os.path.join(fs, p)))
        self.assertEqual(verdict("model-statement-INSERT-22008/verdict.json")["verdict"], "known:btree-gist-infinite-date")
        self.assertEqual(pt.label(verdict("plans-statement-VACUUM-XX001/verdict.json")), "not-reproduced:needs-concurrency:toast-missing-chunk-vacuum-full")
        self.assertEqual(verdict("norec-norec__results_differ/verdict.json")["verdict"], "new")
        self.assertEqual(verdict("crashes/signal-9/verdict.json")["verdict"], "new")
        self.assertEqual(verdict("memory/w2-1700000200.verdict.json")["verdict"], "not-reproduced")
        self.assertEqual(verdict("timeouts/w1-77/verdict.json")["verdict"], "slow-query")
        self.assertEqual(verdict("generator-panics/w1-1700000300.verdict.json")["verdict"], "harness")
        # Only builds whose status is ok.
        self.assertEqual(set(verdict("model-statement-INSERT-22008/verdict.json")["builds"]), {"master", "btree-gist-infinite-date"})
        # The sql-error finding reproduced with psql: patched builds replayed with psql too, never the oracle.
        calls = [c for c in FakeReplayer.calls if c[0] == "model-statement-INSERT-22008"]
        self.assertEqual(calls, [("model-statement-INSERT-22008", "master", "psql"), ("model-statement-INSERT-22008", "btree-gist-infinite-date", "psql")])
        # The oracle-only finding: the oracle on every build.
        self.assertIn(("norec-norec__results_differ", "btree-gist-infinite-date", "oracle"), FakeReplayer.calls)
        # Not reproduced on master: no patched build is tried, both methods on master are.
        toast = [c for c in FakeReplayer.calls if c[0] == "plans-statement-VACUUM-XX001"]
        self.assertEqual(toast, [("plans-statement-VACUUM-XX001", "master", "psql"), ("plans-statement-VACUUM-XX001", "master", "oracle")])
        # A line per finding in deploy.log and pretriage.jsonl.
        log = open(os.path.join(self.root, "deploy.log")).read().splitlines()
        self.assertEqual(len(log), 7)
        self.assertTrue(any("model-statement-INSERT-22008 known:btree-gist-infinite-date" in l for l in log))
        self.assertEqual(len(open(os.path.join(self.run, "pretriage.jsonl")).read().splitlines()), 7)
        # A second pass has nothing to do.
        FakeReplayer.calls = []
        self.assertEqual(pt.main(self.args()), 0)
        self.assertEqual(FakeReplayer.calls, [])
        self.assertEqual(len(open(os.path.join(self.root, "deploy.log")).read().splitlines()), 7)

    def test_master_bin_override_and_extra_build(self):
        ns = types.SimpleNamespace(builds=self.builds, master_bin="/opt/pg/bin", build=["extra=/opt/x/bin"])
        builds, commit, _ = pt.load_builds(ns, REGISTRY)
        self.assertEqual(list(builds), ["master", "btree-gist-infinite-date", "extra"])
        self.assertEqual(builds["master"], "/opt/pg/bin")
        self.assertTrue(commit.startswith("158b6d5109fa"))   # /opt/pg is no checkout: the builds' commit stands

    def test_no_master_build(self):
        os.remove(os.path.join(self.builds, "status.json"))
        self.assertEqual(pt.main(self.args()), 1)


class Checks(unittest.TestCase):
    def test_status(self):
        bug = next(b for b in REGISTRY["bugs"] if b["id"] == "btree-gist-infinite-date")
        good, bad = (bug["expected"], None, 0.3, 10), ("q1\npsql:<stdin>:6: ERROR:  22008\n", None, 0.3, 10)
        self.assertEqual(pt.check_status(bug, bad, good), "open")
        self.assertEqual(pt.check_status(bug, good, good), "fixed-on-master")
        self.assertEqual(pt.check_status(bug, bad, bad), "fix-broken")
        self.assertEqual(pt.check_status(bug, bad, None), "open-no-build")
        self.assertEqual(pt.check_status(dict(bug, expected=None), bad, good), "no-expected")

    def test_limits(self):
        bug = next(b for b in REGISTRY["bugs"] if b["id"] == "huge-expr-timeout")
        out = bug["expected"]
        # Master prints the same once parse analysis ends: the limits tell it apart.
        self.assertEqual(pt.check_status(bug, (out, "rss", 3.2, 650), (out, None, 0.15, 160)), "open")
        self.assertEqual(pt.check_status(bug, (out, None, 0.15, 160), (out, None, 0.15, 160)), "fixed-on-master")
        self.assertFalse(pt.check_ok(bug, (out, None, 31.0, 100)))


class FakeBackendCluster:
    """A cluster whose one 'backend' is a sleep process with a given RSS."""

    def __init__(self, rss_kb):
        self.proc = subprocess.Popen(["sleep", "30"])
        self.rss_kb, self.killed = rss_kb, []

    def env(self):
        return dict(os.environ)

    def backends(self):
        return [(self.proc.pid, self.rss_kb)] if self.proc.poll() is None else []


class Watchdog(unittest.TestCase):
    def test_memory_kill(self):
        c = FakeBackendCluster(5000 * 1024)
        rc, out, killed, elapsed, peak = pt.run_watched(["sleep", "3"], c, 4096, 20, poll=0.05)
        self.assertEqual(killed, "rss")
        self.assertEqual(c.killed, [c.proc.pid])
        self.assertEqual(peak, 5000)
        c.proc.wait()

    def test_time_limit(self):
        c = FakeBackendCluster(10)
        t0 = time.time()
        rc, out, killed, elapsed, peak = pt.run_watched(["sleep", "30"], c, 4096, 1, poll=0.05)
        self.assertEqual(killed, "time")
        self.assertLess(time.time() - t0, 10)
        c.proc.wait()

    def test_output_and_stdin(self):
        c = FakeBackendCluster(10)
        rc, out, killed, _, _ = pt.run_watched(["cat"], c, 4096, 10, stdin="hello\n", poll=0.05)
        self.assertEqual((rc, out, killed), (0, "hello\n", None))
        c.proc.kill()
        c.proc.wait()


if __name__ == "__main__":
    unittest.main()
