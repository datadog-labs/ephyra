#!/usr/bin/env bash
# usage: repro.sh <path-to-datafusion-cli | build-dir containing bin/ or target/*/datafusion-cli>
# exit 1 = bug reproduces, 0 = does not, 2 = cannot decide
# Two INSERTs give the table two batches. ORDER BY x, y LIMIT 2 must keep (-0.0, 5):
# it has the same x as (-0.0, 7) and a smaller y. The bug drops it.
cd "$(dirname "$0")" || exit 2
CLI=$1
[ -d "$CLI" ] && for c in "$CLI/bin/datafusion-cli" "$CLI"/target/*/datafusion-cli; do [ -x "$c" ] && CLI=$c && break; done
[ -x "$CLI" ] && [ ! -d "$CLI" ] || { echo "datafusion-cli not found" >&2; exit 2; }
OUT=$("$CLI" -q -f repro.sql 2>&1) || { echo "$OUT" >&2; exit 2; }
echo "$OUT"
# last result table = the LIMIT query
LAST=$(echo "$OUT" | awk '/^\| x/{buf=""} {buf=buf $0 "\n"} END{printf "%s", buf}')
if echo "$LAST" | grep -Eq '^\| -0\.0 +\| 5 +\|'; then exit 0; fi
if echo "$LAST" | grep -Eq '^\| -0\.0 +\| 7 +\|'; then exit 1; fi
exit 2
