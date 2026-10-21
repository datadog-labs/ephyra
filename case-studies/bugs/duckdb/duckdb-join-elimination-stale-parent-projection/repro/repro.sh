#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = bug reproduces (INTERNAL error), 0 = query returns the expected rows, 2 = cannot decide
DUCKDB=${1:-duckdb}
cd "$(dirname "$0")" || exit 2
out=$("$DUCKDB" -no-agent -csv -noheader :memory: < repro.sql 2>&1)
echo "$out"
if grep -q 'Projection map references a missing child column' <<<"$out"; then exit 1; fi
if [ "$(sort <<<"$out" | tr '\n' ' ')" = "1 2 " ]; then exit 0; fi
exit 2
