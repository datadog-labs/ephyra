#!/usr/bin/env bash
# run.sh [python]: run poc.py with a python that can import litellm
# (e.g. a venv with `pip install litellm`, or PYTHONPATH=<litellm checkout>).
# Exit 1 if the bug shows (stream and non-stream finish_reason differ), 0 if not.
cd "$(dirname "$0")" || exit 2
PY=${1:-python3}
out=$(LITELLM_LOCAL_MODEL_COST_MAP=True NO_PROXY='*' "$PY" poc.py) || exit 2
echo "$out"
ns=$(echo "$out" | sed -n 's/^non-stream finish_reason: *//p')
st=$(echo "$out" | sed -n 's/^stream finish_reason: *//p')
[ -n "$ns" ] && [ -n "$st" ] || exit 2
[ "$ns" = "$st" ] && exit 0 || exit 1
