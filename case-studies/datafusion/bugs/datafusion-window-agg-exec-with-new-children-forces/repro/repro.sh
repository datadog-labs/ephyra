#!/usr/bin/env bash
# usage: repro.sh [path-to-datafusion-cli]
# exit 1 = bug reproduces (SanityCheckPlan error), 0 = query returns 2 rows, 2 = cannot decide
cd "$(dirname "$0")" || exit 2
cli="${1:-datafusion-cli}"
command -v "$cli" >/dev/null 2>&1 || [ -x "$cli" ] || { echo "datafusion-cli not found" >&2; exit 2; }
out=$("$cli" -q --format csv -f repro.sql 2>&1)
echo "$out"
if grep -q "does not satisfy distribution requirements" <<<"$out"; then exit 1; fi
[ "$(grep -cx '2,1' <<<"$out")" -eq 2 ] && exit 0
exit 2
