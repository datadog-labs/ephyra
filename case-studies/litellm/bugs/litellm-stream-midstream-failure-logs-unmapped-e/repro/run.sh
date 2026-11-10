#!/usr/bin/env bash
# run.sh [python]: runs the PoCs with a python that has litellm installed (default: python3).
# poc.py / poc_async.py exit 1 when the bug reproduces (logged error_class != raised class), 0 when not.
# No network: the provider is a local mock server.
PY=${1:-python3}
cd "$(dirname "$0")" || exit 2
export LITELLM_LOCAL_MODEL_COST_MAP=True NO_PROXY='*'
for f in poc.py poc_async.py; do
  echo "== $f"; "$PY" -I "$f" 2>/dev/null | grep -E '^(raised|logged):'; echo "exit ${PIPESTATUS[0]}"
done
for a in "" stream; do
  echo "== contrast.py $a"; "$PY" -I contrast.py $a 2>/dev/null | grep -E '^(raised|logged):'
done
