#!/bin/bash
# usage: repro.sh /path/to/duckdb-cli
# exit 1 = bug reproduces, 0 = does not, 2 = cannot decide
DUCKDB=${1:-duckdb}
case "$DUCKDB" in */*) DUCKDB=$(realpath "$DUCKDB") || exit 2;; esac
command -v "$DUCKDB" >/dev/null 2>&1 || [ -x "$DUCKDB" ] || { echo "duckdb CLI not found: $DUCKDB" >&2; exit 2; }
cd "$(dirname "$0")" || exit 2
out=$(AI_AGENT= "$DUCKDB" -csv < repro.sql 2>&1)
echo "$out" | grep -v -e '\[0x' -e '^$'
if echo "$out" | grep -q 'tuple width should not be 0'; then exit 1; fi
if echo "$out" | grep -q '^42$'; then exit 0; fi
exit 2
