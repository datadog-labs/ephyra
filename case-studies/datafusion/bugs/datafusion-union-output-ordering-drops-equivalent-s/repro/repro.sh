#!/usr/bin/env bash
# usage: repro.sh [path/to/datafusion-cli]
# exit 1 = bug reproduces (SanityCheckPlan error), 0 = query returns rows, 2 = cannot decide
cd "$(dirname "$0")" || exit 2
CLI="${1:-datafusion-cli}"
out=$("$CLI" -q -f repro.sql 2>&1)
echo "$out"
if grep -q 'does not satisfy order requirements' <<<"$out"; then exit 1; fi
if grep -q 'SanityCheckPlan\|Error' <<<"$out"; then exit 2; fi
exit 0
