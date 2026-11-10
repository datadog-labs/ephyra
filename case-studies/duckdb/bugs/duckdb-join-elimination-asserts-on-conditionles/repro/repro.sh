#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# Needs a build with assertions (e.g. `make relassert` or `make debug`).
# exit 1 = INTERNAL error reproduced, 0 = query returned the expected 3 rows of 0, 2 = cannot decide
DUCKDB=${1:?usage: repro.sh <path-to-duckdb-cli>}
cd "$(dirname "$0")" || exit 2
[ -x "$DUCKDB" ] || exit 2
out=$(env -u AI_AGENT -u CLAUDECODE timeout 120 "$DUCKDB" -csv :memory: < repro.sql 2>&1)
rc=$?
if echo "$out" | grep -q 'INTERNAL Error'; then echo "$out" | grep 'INTERNAL Error'; exit 1; fi
[ $rc -eq 0 ] || { echo "rc=$rc: $out"; exit 2; }
[ "$out" = $'r\n0\n0\n0' ] && { echo "ok: 3 rows of 0"; exit 0; }
echo "unexpected output: $out"; exit 2
