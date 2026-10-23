#!/usr/bin/env python3
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Greedy reducer for generated programs: drops top-level items, then
statement lines, while `test.sh <file>` keeps exiting 1.
usage: reduce.py prog.ml test.sh out.ml"""
import subprocess, sys, re, os, tempfile
src, test, out = sys.argv[1:4]
text = open(src).read()
def ok(t):
    with tempfile.NamedTemporaryFile('w', suffix='.ml', delete=False, dir=os.path.dirname(os.path.abspath(out))) as f:
        f.write(t); n = f.name
    try:
        r = subprocess.run(['bash', test, n], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=300)
        return r.returncode == 1
    except subprocess.TimeoutExpired:
        return False
    finally:
        os.unlink(n)
assert ok(text), "initial program does not fail"
def items(t):
    parts = re.split(r'(?m)^(?=let |type |exception |module )', t)
    return [p for p in parts if p]
changed = True
while changed:
    changed = False
    its = items(text)
    i = len(its) - 1
    while i >= 0:
        cand = ''.join(its[:i] + its[i+1:])
        if ok(cand):
            its = its[:i] + its[i+1:]; text = cand; changed = True
        i -= 1
    lines = text.split('\n')
    i = len(lines) - 1
    while i >= 0:
        if lines[i].startswith('  '):
            cand = '\n'.join(lines[:i] + lines[i+1:])
            if ok(cand):
                lines = lines[:i] + lines[i+1:]; text = cand; changed = True
        i -= 1
    open(out, 'w').write(text)
open(out, 'w').write(text)
print(len(text.split('\n')), 'lines')
