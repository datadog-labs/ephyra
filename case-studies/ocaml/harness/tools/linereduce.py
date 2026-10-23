#!/usr/bin/env python3
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Greedy line/chunk deletion reducer for one file.
usage: linereduce.py <file> <test-cmd>   (test-cmd exit 0 = still interesting; run in file's dir)"""
import subprocess, sys
f, cmd = sys.argv[1], sys.argv[2]
lines = open(f).read().split('\n')
def ok(ls):
    open(f, 'w').write('\n'.join(ls))
    return subprocess.run(cmd, shell=True, capture_output=True).returncode == 0
assert ok(lines), "initial not interesting"
n = max(1, len(lines) // 2)
while n >= 1:
    i = 0; changed = False
    while i < len(lines):
        cand = lines[:i] + lines[i+n:]
        if ok(cand):
            lines = cand; changed = True
        else:
            i += n
    if not changed:
        n //= 2
ok(lines)
print(len(lines), "lines")
