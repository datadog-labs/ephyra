#!/usr/bin/env python3
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Line coverage of bbtree's sources: the harness against bbtree's own tests.

usage: cov-compare.py <llvm-bin-dir> <out-dir> harness=<profraw-dir>:<target-dir> upstream=<profraw-dir>:<target-dir>

Merges each side's profiles, exports lcov for every test binary in its
target dir, and prints per-file line coverage for bbtree/{src,
bbtree-store/src, bbtree-mmap/src}, plus the lines only one side reached.
"""
import glob
import json
import os
import subprocess
import sys
from collections import defaultdict

llvm, out = sys.argv[1], sys.argv[2]
sides = dict(a.split("=", 1) for a in sys.argv[3:])
SRC = ("/bbtree/bbtree/src/", "/bbtree/bbtree-store/src/", "/bbtree/bbtree-mmap/src/")


def binaries(target):
    found = []
    for path in glob.glob(os.path.join(target, "**", "deps", "*"), recursive=True):
        if os.path.isfile(path) and os.access(path, os.X_OK) and "." not in os.path.basename(path):
            found.append(path)
    return found


def lines(name, spec):
    profraw, target = spec.split(":")
    profdata = os.path.join(out, f"{name}.profdata")
    raws = glob.glob(os.path.join(profraw, "*.profraw"))
    subprocess.run([f"{llvm}/llvm-profdata", "merge", "-sparse", "-o", profdata, *raws], check=True)
    hit, seen = defaultdict(set), defaultdict(set)
    for binary in binaries(target):
        r = subprocess.run(
            [f"{llvm}/llvm-cov", "export", "-format=lcov", f"-instr-profile={profdata}", binary],
            capture_output=True, text=True,
        )
        current = None
        for line in r.stdout.splitlines():
            if line.startswith("SF:"):
                path = line[3:]
                current = next((path[path.index(s) + 1:] for s in SRC if s in path), None)
            elif line.startswith("DA:") and current:
                n, count = line[3:].split(",")[:2]
                seen[current].add(int(n))
                if int(count) > 0:
                    hit[current].add(int(n))
    return hit, seen


results = {name: lines(name, spec) for name, spec in sides.items()}
(h_hit, h_seen), (u_hit, u_seen) = results["harness"], results["upstream"]
files = sorted(set(h_seen) | set(u_seen))
rows = []
for f in files:
    total = len(h_seen[f] | u_seen[f])
    if not total or "/tests" in f or f.endswith("tests.rs") or "test_support" in f or "shuttle" in f:
        continue
    only_h, only_u = h_hit[f] - u_hit[f], u_hit[f] - h_hit[f]
    rows.append((f, total, len(h_hit[f]), len(u_hit[f]), len(only_h), sorted(only_h), len(only_u)))

print(f"{'file':55} {'lines':>6} {'harness':>8} {'upstream':>9} {'h-only':>7} {'u-only':>7}")
for f, total, h, u, oh, _, ou in rows:
    if h or u:
        print(f"{f:55} {total:6} {h:8} {u:9} {oh:7} {ou:7}")
tot = [sum(r[i] for r in rows) for i in (1, 2, 3, 4, 6)]
print(f"{'TOTAL':55} {tot[0]:6} {tot[1]:8} {tot[2]:9} {tot[3]:7} {tot[4]:7}")
json.dump({r[0]: r[5] for r in rows if r[4]}, open(os.path.join(out, "harness-only-lines.json"), "w"), indent=1)
json.dump(
    {f: sorted(u_hit[f] - h_hit[f]) for f, *_ in rows if u_hit[f] - h_hit[f]},
    open(os.path.join(out, "upstream-only-lines.json"), "w"), indent=1,
)
