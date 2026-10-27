#!/bin/sh
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# show.sh <finding-dir>...: minimized program (imports dropped) and messages.
for d in "$@"; do
  echo "=== $d"
  grep -v "^import typing\|^from ty_ext\|^import ty_ext\|^from typing\|^from _rt\|^from collections\|^import contextlib\|^import enum\|^from dataclasses\|^from __future" "$d/min_check.py" 2>/dev/null || cat "$d/min.py"
  python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print('>>', d['min_msg']); [print('  err', e[:160]) for e in d.get('errs', [])]" "$d/finding.json"
done
