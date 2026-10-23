#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = error reproduced (assertion build: INTERNAL Error; release build: Invalid Error: unordered_map::at),
# 0 = correct result (15), 2 = cannot decide
DUCKDB=$(realpath "${1:?usage: repro.sh <path-to-duckdb-cli>}") || exit 2
cd "$(dirname "$0")" || exit 2
[ -x "$DUCKDB" ] || exit 2
out=$(env -u AI_AGENT -u CLAUDECODE timeout 120 "$DUCKDB" -csv :memory: < repro.sql 2>&1)
echo "$out" | head -3
case "$out" in
  *"INTERNAL Error"*|*"unordered_map::at"*) exit 1 ;;
  $'count_star()\n15') exit 0 ;;
  *) exit 2 ;;
esac
