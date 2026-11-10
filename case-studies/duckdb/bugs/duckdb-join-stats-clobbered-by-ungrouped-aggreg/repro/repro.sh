#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = wrong result (LIMIT 1 returns 3 rows), 0 = correct (1 row), 2 = could not run
D=$(realpath "${1:?usage: repro.sh <path-to-duckdb-cli>}") || exit 2
cd "$(dirname "$0")" || exit 2
out=$(env -u AI_AGENT -u CLAUDECODE "$D" -noheader -csv < repro.sql 2>&1)
echo "$out"
case "$out" in
  1) exit 0 ;;
  3) exit 1 ;;
  *) exit 2 ;;
esac
