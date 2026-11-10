#!/usr/bin/env bash
# repro.sh <build-dir>: exit 1 if the bug reproduces, 0 if not, 2 on setup error.
# Without a build dir, runs poc.py with `python3` (needs `pip install litellm` or PYTHONPATH=<litellm checkout>).
if [ -n "${1:-}" ]; then
  B=$(cd "$1" && pwd) || exit 2
  source "$B/env.sh" || exit 2
  PY=$LITELLM_PY
else
  PY=python3
fi
cd "$(dirname "$0")" || exit 2
NO_PROXY='*' "$PY" poc.py
rc=$?
[ $rc -le 1 ] && exit $rc || exit 2
