#!/bin/bash
# usage: repro.sh <path-to-duckdb-cli>
# Runs crash.sql and wrong_result.sql, each in a fresh in-memory database.
# Exit 1 if either bug reproduces (INTERNAL error / wrong rows), 0 if both queries give the
# expected result, 2 otherwise (e.g. a version that rejects the query).
D=${1:-duckdb}
case "$D" in */*) D=$(realpath "$D") || exit 2;; *) D=$(command -v "$D") || exit 2;; esac
cd "$(dirname "$0")" || exit 2
run() { env -u AI_AGENT -u CLAUDECODE "$D" -csv -noheader < "$1" 2>&1; }
status=0
out=$(run crash.sql); rc=$?
echo "== crash.sql"; echo "$out" | head -3
if grep -q 'INTERNAL Error' <<<"$out"; then echo "crash.sql: REPRODUCES"; status=1
elif [ $rc -eq 0 ] && [ "$(tail -n1 <<<"$out")" = 50 ]; then echo "crash.sql: OK (50)"
else echo "crash.sql: UNDECIDED (rc=$rc)"; status=2; fi
out=$(run wrong_result.sql); rc=$?
echo "== wrong_result.sql"; echo "$out" | head -12
if grep -q 'INTERNAL Error' <<<"$out"; then echo "wrong_result.sql: INTERNAL error"; status=1
elif [ $rc -eq 0 ] && [ "$out" = "$(cat wrong_result.expected)" ]; then echo "wrong_result.sql: OK"
elif [ $rc -eq 0 ]; then echo "wrong_result.sql: REPRODUCES (rows differ from wrong_result.expected)"; status=1
else echo "wrong_result.sql: UNDECIDED (rc=$rc)"; [ $status -eq 0 ] && status=2; fi
exit $status
