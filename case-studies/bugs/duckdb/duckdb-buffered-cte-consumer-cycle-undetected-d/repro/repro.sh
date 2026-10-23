#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = query did not finish within 30 s (hang reproduces), 0 = query finished, 2 = other error
D=${1:-duckdb}
cd "$(dirname "$0")" || exit 2
timeout 30 "$D" -batch -init /dev/null -csv -c ".read repro.sql"
rc=$?
[ $rc = 124 ] && { echo "TIMEOUT after 30 s"; exit 1; }
[ $rc = 0 ] && exit 0
exit 2
