#!/bin/sh
# Usage: run.sh <python-with-fastmcp>   (e.g. .venv/bin/python)
# Prints "matches_allowed_pattern: True" when the bug is present.
# Optional browser-side check: node -e 'console.log(new URL("https://evil.com\\.example.com/cb").hostname)'  -> evil.com
PY=${1:-python}
exec "$PY" -I "$(dirname "$0")/redirect_backslash.py"
