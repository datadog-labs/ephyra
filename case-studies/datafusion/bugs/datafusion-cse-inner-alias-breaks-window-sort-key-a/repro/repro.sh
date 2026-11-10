#!/usr/bin/env bash
# usage: repro.sh <datafusion-cli binary>
# The binary must be built with debug assertions (the default `cargo build -p datafusion-cli`,
# or the `ci` / `ci-optimized` profiles). Release builds skip the assertion and return the right rows.
# Exit 1 = bug reproduces (panic "all window expressions shall have the same sort keys"),
#      0 = query returns the expected rows, 2 = could not decide.
set -u
CLI=${1:?usage: repro.sh <datafusion-cli binary>}
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=$("$CLI" -q --format csv -f "$HERE/repro.sql" 2>&1)
echo "$OUT"
if grep -q "all window expressions shall have the same sort keys" <<<"$OUT"; then exit 1; fi
# expected rows: 2,10 and 4,30
if grep -qx "2,10" <<<"$OUT" && grep -qx "4,30" <<<"$OUT"; then exit 0; fi
exit 2
