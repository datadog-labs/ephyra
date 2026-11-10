#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = bug reproduces (INTERNAL Error), 0 = query runs, 2 = cannot decide
D=${1:-duckdb}; [ -e "$D" ] && D=$(realpath "$D")
command -v "$D" >/dev/null 2>&1 || [ -x "$D" ] || { echo "no duckdb binary: $D"; exit 2; }
cd "$(dirname "$0")" || exit 2
# -no-agent keeps the default box output if AI_AGENT is set; older releases lack the flag
if "$D" -no-agent -c "select 1" >/dev/null 2>&1; then F=-no-agent; else F=; fi
out=$("$D" $F < repro.sql 2>&1)
echo "$out" | head -5
if echo "$out" | grep -q 'INTERNAL Error'; then exit 1; fi
if echo "$out" | grep -q '0 rows'; then exit 0; fi
exit 2
