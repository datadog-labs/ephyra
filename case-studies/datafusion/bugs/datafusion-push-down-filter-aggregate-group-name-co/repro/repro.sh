#!/usr/bin/env bash
# usage: repro.sh /path/to/datafusion-cli
# exit 1 = bug (type error at execution), 0 = correct (one row, a = 2), 2 = cannot decide
cli=${1:?usage: repro.sh /path/to/datafusion-cli}
cd "$(dirname "$0")" || exit 2
out=$("$cli" -q --format csv -f repro.sql 2>&1)
echo "$out"
if grep -q "Invalid comparison operation" <<<"$out"; then exit 1; fi
if [ "$(tail -n 2 <<<"$out" | tr '\n' ' ')" = "a 2 " ]; then exit 0; fi
exit 2
