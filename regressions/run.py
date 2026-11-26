#!/usr/bin/env python3
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.
"""Re-inject fixed bugs and check that the tests catch them (regressions/README.md).

    python3 regressions/run.py                 every entry: fix soundness, then bug soundness
    python3 regressions/run.py ID ...          only these entries
    python3 regressions/run.py --matrix        also run every entry's oracles on every injection
    python3 regressions/run.py --check         only check that every injection applies and reverses
    python3 regressions/run.py --json FILE     write the results as JSON too

The library is copied (crates/, the workspace files) to a scratch directory, so the working
tree is never touched and uncommitted changes are tested as they are. Builds share
target/regressions.
"""

import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
HERE = ROOT / "regressions"
COPY = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo", "crates"]
TARGET_DIR = ROOT / "target" / "regressions"
RESULT = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)", re.M)


def entries(ids):
    found = sorted(p.parent for p in HERE.glob("*/meta.json"))
    if ids:
        missing = set(ids) - {p.name for p in found}
        if missing:
            sys.exit(f"no such entries: {', '.join(sorted(missing))}")
        found = [p for p in found if p.name in ids]
    out = []
    for d in found:
        meta = json.loads((d / "meta.json").read_text())
        if meta.get("id") != d.name:
            sys.exit(f"{d.name}/meta.json: id is {meta.get('id')!r}")
        if not meta.get("oracles"):
            sys.exit(f"{d.name}/meta.json: no oracles (the class properties that must catch it)")
        meta["dir"] = d
        out.append(meta)
    return out


def copy_tree(dest):
    for name in COPY:
        src = ROOT / name
        if src.is_dir():
            shutil.copytree(src, dest / name, ignore=shutil.ignore_patterns("target", ".ephyra"))
        elif src.exists():
            shutil.copy2(src, dest / name)


def git_apply(tree, patch, *flags):
    return subprocess.run(["git", "apply", *flags, str(patch)], cwd=tree,
                          capture_output=True, text=True)


def digest(tree, files):
    h = hashlib.sha256()
    for f in sorted(files):
        p = tree / f
        h.update(f.encode() + b"\0" + (p.read_bytes() if p.exists() else b"<missing>"))
    return h.hexdigest()


def touched(patch):
    return sorted(set(re.findall(r"^\+\+\+ b/(\S+)", patch.read_text(), re.M)))


def tests_of(meta, kinds=("oracles", "regression")):
    for kind in kinds:
        for t in meta.get(kind, []):
            yield kind, t


def selector(t):
    """The cargo test arguments that select test `t`'s binary."""
    target = t["target"]
    sel = ["--lib"] if target == "lib" else ["--test", target.removeprefix("test:")]
    return ["-p", t.get("package", "ephyra"), *sel, *(["--features", t["features"]] if t.get("features") else [])]


def run_tests(tree, tests, log):
    """Run tests (dicts with package/target/name), grouped by binary. Returns
    ({name: "ok" | "FAILED" | "missing"}, build_ok)."""
    env = dict(os.environ, CARGO_TARGET_DIR=str(TARGET_DIR))
    groups = {}
    for t in tests:
        groups.setdefault(tuple(selector(t)), []).append(t["name"])
    results, built = {}, True
    for sel, names in groups.items():
        build = subprocess.run(["cargo", "test", "--quiet", "--no-run", *sel], cwd=tree, env=env,
                               capture_output=True, text=True)
        if build.returncode != 0:
            log.write(build.stderr)
            built = False
            for n in names:
                results[n] = "build failed"
            continue
        # every name in one run: libtest takes several filters; --exact keeps them exact
        run = subprocess.run(["cargo", "test", *sel, "--", "--exact", "--test-threads=4", *names],
                             cwd=tree, env=env, capture_output=True, text=True)
        log.write(run.stdout + run.stderr)
        seen = dict(RESULT.findall(run.stdout))
        for n in names:
            results[n] = seen.get(n, "missing")
    return results, built


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("ids", nargs="*")
    ap.add_argument("--matrix", action="store_true")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--json")
    ap.add_argument("--keep", action="store_true", help="keep the scratch copy and its logs")
    args = ap.parse_args()
    metas = entries(args.ids)
    scratch = pathlib.Path(tempfile.mkdtemp(prefix="ephyra-regressions-"))
    tree = scratch / "tree"
    tree.mkdir()
    copy_tree(tree)
    log = open(scratch / "log.txt", "w")
    problems, report = [], {"entries": {}, "clean": {}}

    # 1. Every injection applies to the tree as it is and reverses back to it byte for byte.
    for m in metas:
        patch = m["dir"] / "injection.patch"
        files = touched(patch)
        before = digest(tree, files)
        r = git_apply(tree, patch)
        if r.returncode != 0:
            problems.append(f"{m['id']}: injection.patch does not apply: {r.stderr.strip()}")
            m["applies"] = False
            continue
        r = git_apply(tree, patch, "-R")
        if r.returncode != 0 or digest(tree, files) != before:
            problems.append(f"{m['id']}: injection.patch does not reverse cleanly")
            m["applies"] = False
            continue
        m["applies"] = True
        if any(f.split("/")[-1].startswith("test") or "/tests/" in f for f in files):
            problems.append(f"{m['id']}: injection.patch touches a test file ({files})")
    print(f"{sum(m['applies'] for m in metas)}/{len(metas)} injections apply and reverse")
    if args.check:
        return finish(problems, report, args, scratch, log)

    every = {t["name"]: t for m in metas for _, t in tests_of(m)}
    # 2. Fix soundness: on the clean tree every oracle and regression test passes.
    t0 = time.time()
    clean, built = run_tests(tree, every.values(), log)
    report["clean"] = clean
    bad = {n: r for n, r in clean.items() if r != "ok"}
    print(f"clean tree: {len(clean) - len(bad)}/{len(clean)} tests pass ({time.time() - t0:.0f}s)")
    for n, r in bad.items():
        problems.append(f"clean tree: {n} is {r}")

    # 3. Bug soundness: with an entry's injection, each of its oracles fails.
    for m in metas:
        if not m["applies"]:
            continue
        patch = m["dir"] / "injection.patch"
        git_apply(tree, patch)
        try:
            chosen = every.values() if args.matrix else [t for _, t in tests_of(m)]
            t0 = time.time()
            res, built = run_tests(tree, chosen, log)
        finally:
            git_apply(tree, patch, "-R")
        own = {t["name"] for _, t in tests_of(m, ("oracles",))}
        caught = sorted(n for n, r in res.items() if r == "FAILED")
        missed = sorted(n for n in own if res.get(n) != "FAILED")
        report["entries"][m["id"]] = {"results": res, "caught_by": caught, "built": built}
        status = "CAUGHT" if not missed else ("BUILD FAILED" if not built else "MISSED")
        print(f"{m['id']}: {status} ({time.time() - t0:.0f}s); failing: {', '.join(caught) or 'none'}")
        for n in missed:
            problems.append(f"{m['id']}: oracle {n} is {res.get(n)} with the injection (must fail)")
    return finish(problems, report, args, scratch, log)


def finish(problems, report, args, scratch, log):
    log.close()
    report["problems"] = problems
    if args.json:
        pathlib.Path(args.json).write_text(json.dumps(report, indent=1) + "\n")
    if args.matrix and report["entries"]:
        names = sorted({n for e in report["entries"].values() for n in e["results"]})
        print("\nmatrix (X: fails with the injection)")
        for eid, e in report["entries"].items():
            print(f"  {eid}")
            for n in names:
                if e["results"].get(n) == "FAILED":
                    print(f"    X {n}")
    for p in problems:
        print("PROBLEM:", p)
    if args.keep or problems:
        print(f"scratch and log: {scratch}")
    else:
        shutil.rmtree(scratch)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
