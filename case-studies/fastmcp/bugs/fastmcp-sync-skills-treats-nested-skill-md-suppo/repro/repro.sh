#!/bin/sh
# Usage: repro.sh <python-with-fastmcp>   (e.g. a venv's bin/python, or PYTHONPATH=<checkout>/fastmcp_slim)
# exit 1 = bug reproduces (sync_skills fails / phantom skill listed), 0 = fixed, 2 = cannot decide
PY=${1:-python3}
cd "$(dirname "$0")" || exit 2
out=$("$PY" repro.py 2>&1)
case "$out" in
  *"cannot import name 'Skill'"*) echo "no fastmcp.Skill in this version"; exit 2;;
  *"Resource not found: 'skill://a/sub/_manifest'"*|*"'a/sub'"*) echo "$out" | tail -2; exit 1;;
esac
case "$out" in
  "['a']"*) echo "$out"; exit 0;;
esac
echo "$out" | tail -3; exit 2
