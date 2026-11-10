#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = assertion fires, 0 = query returns normally, 2 = cannot decide
D=${1:-duckdb}
cd "$(dirname "$0")" || exit 2
out=$(env -u AI_AGENT -u CLAUDECODE "$D" :memory: < poc.sql 2>&1)
echo "$out" | head -3
if grep -q 'query_graph_manager.cpp.\{0,4\} on line [0-9]*: normalized' <<<"$out"; then exit 1; fi
if grep -qE 'INTERNAL Error|Error:' <<<"$out"; then exit 2; fi
exit 0
