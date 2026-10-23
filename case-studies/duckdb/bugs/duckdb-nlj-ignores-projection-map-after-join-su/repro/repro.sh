#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# Runs repro.sql in an in-memory database. Exit 1 if the INTERNAL error reproduces, 0 if the query
# returns 3 rows, 2 otherwise.
D=${1:-duckdb}
case "$D" in */*) D=$(realpath "$D") || exit 2;; *) D=$(command -v "$D") || exit 2;; esac
cd "$(dirname "$0")" || exit 2
out=$(env -u AI_AGENT -u CLAUDECODE "$D" -csv -noheader < repro.sql 2>&1); rc=$?
echo "$out"
if grep -q 'INTERNAL Error' <<<"$out"; then echo "REPRODUCES"; exit 1; fi
if [ $rc -eq 0 ] && [ "$(grep -c '^1$' <<<"$out")" = 3 ]; then echo "OK: 3 rows"; exit 0; fi
echo "UNDECIDED (rc=$rc)"; exit 2
