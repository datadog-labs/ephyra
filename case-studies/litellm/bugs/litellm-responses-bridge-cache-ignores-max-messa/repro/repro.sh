#!/usr/bin/env bash
# repro.sh [python]: exit 1 if the bug reproduces, 0 if not, 2 if it cannot decide.
# python: an interpreter that can `import litellm` (default: python3). No network needed.
PY=${1:-python3}
cd "$(dirname "$0")" || exit 2
LITELLM_LOCAL_MODEL_COST_MAP=True NO_PROXY='*' "$PY" poc.py
rc=$?
[ $rc -le 1 ] && exit $rc || exit 2
