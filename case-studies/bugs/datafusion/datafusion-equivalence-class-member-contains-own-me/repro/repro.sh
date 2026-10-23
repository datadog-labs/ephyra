#!/usr/bin/env bash
# usage: repro.sh <path-to-datafusion-cli>
# exit 1 = bug reproduces (SanityCheckPlan error), 0 = query returns the expected row, 2 = cannot decide
cli=${1:?usage: repro.sh <path-to-datafusion-cli>}
here=$(cd "$(dirname "$0")" && pwd)
# run from an empty directory: datafusion-cli treats a bare table name like `t` as a file path if such a file exists
tmp=$(mktemp -d) || exit 2
trap 'rm -rf "$tmp"' EXIT
out=$(cd "$tmp" && "$cli" -q -f "$here/repro.sql" 2>&1)
printf '%s\n' "$out" | cut -c1-300
if printf '%s' "$out" | grep -q 'SanityCheckPlan'; then exit 1; fi
if printf '%s' "$out" | grep -Eq '^\| 1 +\| 0 +\| 1 +\| 0 +\|$'; then exit 0; fi
exit 2
