#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = reproduces (exported SQL fails to rebind), 0 = does not, 2 = cannot decide
D=${1:?usage: repro.sh <duckdb>}
cd "$(dirname "$0")" || exit 2
out=$(env -u AI_AGENT "$D" -csv < repro.sql 2>&1) || true
echo "$out"
grep -q 'Unimplemented explain type\|debug_verify_statement' <<<"$out" && exit 2
grep -q 'must be a constant expression' <<<"$out" && exit 1
grep -q '^1,0.5$' <<<"$out" || exit 2
exit 0
