#!/bin/bash
# usage: run.sh <path-to-duckdb-cli>
# exit 1 = assertion reproduces, 0 = query ran without the assertion, 2 = cannot decide
D=${1:?usage: run.sh <duckdb-cli>}
cd "$(dirname "$0")" || exit 2
out=$("$D" -no-agent < poc.sql 2>&1) || true
echo "$out" | grep -m1 -o 'Assertion triggered.*probe_data.size()' && exit 1
echo "$out" | grep -q 'INTERNAL' && { echo "$out" | head -3; exit 2; }
echo "no assertion"; exit 0
