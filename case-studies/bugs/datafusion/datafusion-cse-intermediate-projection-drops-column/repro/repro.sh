#!/usr/bin/env bash
# usage: repro.sh <path-to-datafusion-cli>
# exit 1 = bug reproduces, 0 = query returns the expected row, 2 = cannot decide
cli=${1:-datafusion-cli}
cd "$(dirname "$0")" || exit 2
out=$("$cli" -q -f repro.sql 2>&1)
echo "$out"
if echo "$out" | grep -q -E "No field named|panicked"; then exit 1; fi
if echo "$out" | grep -q -E "^\| 1 +\| 2 +\| 4 +\|"; then exit 0; fi
exit 2
