#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = INTERNAL Error reproduced, 0 = correct result [[1]], 2 = cannot decide
DUCKDB=$(realpath "${1:?usage: repro.sh <path-to-duckdb-cli>}") || exit 2
cd "$(dirname "$0")" || exit 2
[ -x "$DUCKDB" ] || exit 2
out=$(env -u AI_AGENT -u CLAUDECODE timeout 120 "$DUCKDB" -csv :memory: < repro.sql 2>&1)
echo "$out" | head -3
case "$out" in
  $'r\n"[[1]]"'|$'r\n[[1]]') exit 0 ;;
  *"BoundReferenceExpression should not be used here yet"*) exit 1 ;;
  *) exit 2 ;;
esac
