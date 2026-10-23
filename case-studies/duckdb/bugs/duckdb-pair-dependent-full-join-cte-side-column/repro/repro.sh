#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = INTERNAL Error reproduces, 0 = query returns the expected single row "1", 2 = other output
D=${1:?usage: repro.sh <path-to-duckdb-cli>}
cd "$(dirname "$0")" || exit 2
OUT=$("$D" -no-agent -csv -noheader :memory: < repro.sql 2>&1)
echo "$OUT"
if echo "$OUT" | grep -q 'INTERNAL Error'; then exit 1; fi
[ "$OUT" = "1" ] && exit 0
exit 2
