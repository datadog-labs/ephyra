#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = INTERNAL error (bug reproduces), 0 = query returns a result, 2 = cannot decide
D=${1:?usage: repro.sh <path-to-duckdb-cli>}
cd "$(dirname "$0")" || exit 2
out=$(env -u AI_AGENT -u CLAUDECODE "$D" -batch -csv < poc.sql 2>&1)
echo "$out" | head -3
if echo "$out" | grep -q "unrecognized configuration parameter"; then exit 2; fi
if echo "$out" | grep -q 'INTERNAL Error'; then exit 1; fi
if echo "$out" | grep -qx '1,2'; then exit 0; fi
exit 2
