#!/bin/sh
# Usage: run.sh <python>   (a Python with fastmcp installed, e.g. from a fastmcp checkout: uv run python)
# Prints "-> ok" for both schemas and "data: Root(name=None)" when fixed.
# Shows "ValidationError" for the second schema and "data: None" when the bug is present.
PY=${1:-python}
FASTMCP_SHOW_SERVER_BANNER=false exec "$PY" "$(dirname "$0")/mre.py"
