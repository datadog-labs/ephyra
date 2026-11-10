#!/usr/bin/env bash
# repro.sh [python]: runs poc.py with the given interpreter (default python3), which must
# be able to import litellm with the proxy extra (an upstream checkout on PYTHONPATH after
# `uv sync --extra proxy`, or `pip install 'litellm[proxy]'`).
# exit 1 = the bug reproduces, 0 = it does not, 2 = setup error.
cd "$(dirname "$0")" || exit 2
PY=${1:-python3}
LITELLM_LOCAL_MODEL_COST_MAP=True NO_PROXY='*' "$PY" poc.py
