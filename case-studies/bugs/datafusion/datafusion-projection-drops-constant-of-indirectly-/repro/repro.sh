#!/usr/bin/env bash
# usage: repro.sh <datafusion-cli binary | datafusion checkout>
# With a checkout, builds datafusion-cli (cargo build -p datafusion-cli) first.
# exit 1 = bug reproduces (SanityCheckPlan error), 0 = correct row returned, 2 = cannot decide
set -u
here="$(cd "$(dirname "$0")" && pwd)"
arg="${1:?usage: repro.sh <datafusion-cli binary | datafusion checkout>}"
if [ -d "$arg" ]; then
  (cd "$arg" && cargo build -p datafusion-cli >&2) || exit 2
  cli="${CARGO_TARGET_DIR:-$arg/target}/debug/datafusion-cli"
else
  cli="$arg"
fi
[ -x "$cli" ] || exit 2
out="$("$cli" -q -f "$here/repro.sql" 2>&1)"
if grep -q 'does not satisfy order requirements' <<<"$out"; then
  grep -o 'does not satisfy order requirements.*' <<<"$out" >&2
  exit 1
fi
# expected result (x, a): one row (2, 1)
rows="$(grep -E '^\| [0-9]' <<<"$out" | tr -d ' ')"
[ "$rows" = "|2|1|" ] && exit 0
echo "$out" >&2
exit 2
