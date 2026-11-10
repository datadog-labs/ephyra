#!/usr/bin/env bash
# repro.sh [python]: exit 1 if litellm maps an OpenAI HTTP 403 to something other than PermissionDeniedError.
# Run with a python that has litellm (and openai) installed, e.g. ./repro.sh /path/to/venv/bin/python
PY=${1:-python3}
cd "$(dirname "$0")" || exit 2
LITELLM_LOCAL_MODEL_COST_MAP=True NO_PROXY='*' "$PY" poc.py
