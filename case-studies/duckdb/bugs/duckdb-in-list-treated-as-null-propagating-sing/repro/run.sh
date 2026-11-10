#!/bin/bash
# usage: run.sh <path-to-duckdb-cli>
# Prints the query result with default settings, then with the deliminator disabled.
# Expected both times: one row (false). Exit 1 if the default run returns no row.
D=${1:?usage: run.sh <path-to-duckdb-cli>}
cd "$(dirname "$0")" || exit 2
"$D" -version
echo "== default"
A=$("$D" -csv < repro.sql) || exit 2
echo "$A"
echo "== SET disabled_optimizers='deliminator'"
(echo "SET disabled_optimizers='deliminator';"; cat repro.sql) | "$D" -csv
[ "$(echo "$A" | tail -n +2)" = "false" ] && exit 0 || exit 1
