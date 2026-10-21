#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# Needs a build with assertions (e.g. `make relassert` or `make debug`); release builds compile D_ASSERT out.
# exit 1 = INTERNAL error reproduced, 0 = query returned 1, 2 = cannot decide
DUCKDB=${1:?usage: repro.sh <path-to-duckdb-cli>}
cd "$(dirname "$0")" || exit 2
[ -x "$DUCKDB" ] || exit 2
out=$(env -u AI_AGENT -u CLAUDECODE timeout 120 "$DUCKDB" -csv -noheader :memory: < repro.sql 2>&1)
rc=$?
if echo "$out" | grep -q 'INTERNAL Error'; then echo "$out" | grep 'INTERNAL Error'; exit 1; fi
[ $rc -eq 0 ] || { echo "rc=$rc: $out"; exit 2; }
[ "$out" = "1" ] && { echo "ok: 1"; exit 0; }
echo "unexpected output: $out"; exit 2
