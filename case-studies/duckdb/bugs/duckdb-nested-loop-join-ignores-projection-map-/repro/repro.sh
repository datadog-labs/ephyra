#!/bin/bash
# usage: repro.sh [path-to-duckdb-cli]
# Runs repro.sql in an in-memory database (assertion-enabled builds, e.g. `make relassert`, show the
# assertion; release builds may show a different internal error or wrong output).
# Exit 1 if an INTERNAL error is raised or the result is wrong, 0 if the expected 4 rows come back, 2 otherwise.
D=${1:-duckdb}
case "$D" in */*) D=$(realpath "$D") || exit 2;; *) D=$(command -v "$D") || exit 2;; esac
cd "$(dirname "$0")" || exit 2
out=$("$D" -no-agent -csv -noheader < repro.sql 2>&1); rc=$?
echo "$out"
if grep -q 'INTERNAL Error' <<<"$out"; then echo "REPRODUCES"; exit 1; fi
if [ $rc -eq 0 ] && [ "$out" = $'1,2\n1,2\n2,1\n2,1' ]; then echo "OK: expected 4 rows"; exit 0; fi
if [ $rc -eq 0 ]; then echo "REPRODUCES (wrong result)"; exit 1; fi
echo "UNDECIDED (rc=$rc)"; exit 2
