#!/bin/sh
# usage: run.sh <python> [<fastmcp_slim dir>]; exit 1 = bug shows, 0 = not, 2 = undecided
cd "$(dirname "$0")" || exit 2
PY=${1:?usage: run.sh <python> [<fastmcp_slim dir>]}
[ -n "$2" ] && export PYTHONPATH="$2"
out=$("$PY" poc.py 2>/dev/null | tail -1) || exit 2
case "$out" in
  "{'x': None}") exit 0 ;;
  "{'a': False}") exit 1 ;;
  *) echo "unexpected: $out" >&2; exit 2 ;;
esac
