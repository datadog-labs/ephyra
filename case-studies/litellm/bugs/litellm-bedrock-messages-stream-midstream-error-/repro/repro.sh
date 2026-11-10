#!/usr/bin/env bash
# repro.sh [python]: runs poc.py with the given python (default: python3) that has litellm installed.
# Exit 1 = bug reproduces (raw BedrockError escapes the /messages stream), 0 = mapped litellm exception.
cd "$(dirname "$0")" || exit 2
PY=${1:-python3}
LITELLM_LOCAL_MODEL_COST_MAP=True "$PY" -I poc.py
