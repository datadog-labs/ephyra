#!/bin/sh
# Usage: run.sh <python-with-fastmcp>   (e.g. .venv/bin/python; fastmcp >= 4.0.0)
# Bug present when the first line ends in "myapp:?code=..." instead of "myapp://?code=...".
PY=${1:-python}
exec "$PY" "$(dirname "$0")/empty_authority.py"
