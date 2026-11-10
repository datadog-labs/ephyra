#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# exit 1 = bug reproduces (rows returned for WHERE NULL), 0 = fixed, 2 = cannot decide
CLI=${1:-duckdb}
command -v "$CLI" >/dev/null 2>&1 || { echo "duckdb CLI not found: $CLI" >&2; exit 2; }
cd "$(dirname "$0")" || exit 2
out=$(env -u AI_AGENT "$CLI" -csv -noheader < repro.sql 2>&1) || { echo "$out" >&2; exit 2; }
echo "output: '${out}'"
if [ -z "$out" ]; then echo "OK: 0 rows (expected)"; exit 0; fi
if [ "$out" = "1,2" ]; then echo "BUG: WHERE NULL returned a row"; exit 1; fi
echo "unexpected output" >&2; exit 2
