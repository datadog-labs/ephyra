#!/usr/bin/env bash
# run.sh [python]: python must import the litellm under test (pip install litellm, or a source checkout).
# Exit 1 = bug reproduces (temperature sent to o3-mini through responses()), 0 = fixed.
PY=${1:-python3}
cd "$(dirname "$0")" || exit 2
"$PY" contrast_chat.py || exit 2
"$PY" poc.py
