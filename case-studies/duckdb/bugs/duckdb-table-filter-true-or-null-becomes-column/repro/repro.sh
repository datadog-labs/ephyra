#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = wrong result reproduced (2 rows), 0 = correct result (1 row), 2 = cannot decide
DUCKDB=$(realpath "${1:?usage: repro.sh <path-to-duckdb-cli>}") || exit 2
cd "$(dirname "$0")" || exit 2
[ -x "$DUCKDB" ] || exit 2
out=$(env -u AI_AGENT -u CLAUDECODE timeout 120 "$DUCKDB" -csv :memory: < repro.sql 2>&1) || { echo "error: $out"; exit 2; }
echo "$out"
case "$out" in
  $'b\ntrue') exit 0 ;;
  $'b\ntrue\nfalse') exit 1 ;;
  *) exit 2 ;;
esac
