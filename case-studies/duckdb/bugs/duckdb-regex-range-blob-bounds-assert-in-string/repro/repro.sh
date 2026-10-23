#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>   (assertion-enabled build, e.g. make relassert or make debug)
# exit 1 = reproduces (INTERNAL Error), 0 = does not, 2 = cannot decide
D=${1:?usage: repro.sh <duckdb>}
cd "$(dirname "$0")" || exit 2
out=$("$D" -csv < repro.sql 2>&1) || true
echo "$out" | head -5
grep -q 'INTERNAL Error' <<<"$out" && exit 1
grep -q '^1,a,\(NULL\)\?$' <<<"$out" && exit 0
exit 2
