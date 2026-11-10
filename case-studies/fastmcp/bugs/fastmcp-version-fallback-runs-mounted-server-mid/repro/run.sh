#!/bin/sh
# run.sh [python-with-fastmcp]: exit 1 = bug shows, 0 = it does not, 2 = could not run.
PY=${1:-python3}
cd "$(dirname "$0")" || exit 2
timeout 60 "$PY" -W ignore mre.py
r=$?
[ $r -le 1 ] && exit $r || exit 2
