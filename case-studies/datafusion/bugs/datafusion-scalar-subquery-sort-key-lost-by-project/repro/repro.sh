#!/usr/bin/env bash
# usage: repro.sh <datafusion-cli binary | datafusion checkout>
# With a checkout, builds datafusion-cli (cargo build --release -p datafusion-cli) first.
# exit 1 = bug reproduces (SanityCheckPlan error), 0 = correct rows returned, 2 = cannot decide
set -u
here="$(cd "$(dirname "$0")" && pwd)"
arg="${1:?usage: repro.sh <datafusion-cli binary | datafusion checkout>}"
if [ -d "$arg" ]; then
  (cd "$arg" && cargo build --release -p datafusion-cli >&2) || exit 2
  cli="$arg/target/release/datafusion-cli"
else
  cli="$arg"
fi
[ -x "$cli" ] || exit 2
out="$("$cli" -q -f "$here/repro.sql" 2>&1)"
if grep -q 'does not satisfy order requirements' <<<"$out"; then
  grep -o 'does not satisfy order requirements.*' <<<"$out" >&2
  exit 1
fi
# expected result (x0, x1): (true,3) (true,3) (false,3)
rows="$(grep -E '^\| (true|false) ' <<<"$out" | tr -d ' ')"
if [ "$rows" = "$(printf '|true|3|\n|true|3|\n|false|3|')" ]; then
  exit 0
fi
echo "$out" >&2
exit 2
