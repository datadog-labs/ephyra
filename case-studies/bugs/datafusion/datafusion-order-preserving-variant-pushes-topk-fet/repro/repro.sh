#!/usr/bin/env bash
# usage: repro.sh [path-to-datafusion-cli]
# exit 1 = bug reproduces (n = 1), 0 = correct (n = 3), 2 = cannot decide
cli="${1:-datafusion-cli}"
case "$cli" in */*) cli="$(cd "$(dirname "$cli")" && pwd)/$(basename "$cli")";; esac
cd "$(dirname "$0")" || exit 2
command -v "$cli" >/dev/null 2>&1 || [ -x "$cli" ] || { echo "datafusion-cli not found" >&2; exit 2; }
out=$("$cli" -q --format csv -f repro.sql 2>&1)
echo "$out"
grep -qx '1,1' <<<"$out" && exit 1
grep -qx '3,1' <<<"$out" && exit 0
exit 2
