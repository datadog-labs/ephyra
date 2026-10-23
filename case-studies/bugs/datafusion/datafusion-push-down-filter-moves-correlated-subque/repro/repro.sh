#!/usr/bin/env bash
# usage: repro.sh <datafusion-cli binary | datafusion checkout>
# With a checkout, builds datafusion-cli (cargo build --release -p datafusion-cli) first.
# exit 1 = bug reproduces (optimizer schema error), 0 = correct rows returned, 2 = cannot decide
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
out="$(printf '%s\n' \
  "CREATE TABLE t1 (c0 BIGINT) AS VALUES (1), (3);" \
  "CREATE TABLE t2 (c0 BIGINT, c1 VARCHAR) AS VALUES (1, 'x'), (2, 'y');" \
  "SELECT * FROM t2 AS a JOIN t1 AS b ON a.c0 = ANY (SELECT c0 FROM t1) ORDER BY b.c0;" \
  | "$cli" -q 2>&1)"
if grep -q 'No field named a.c0' <<<"$out"; then
  grep -o 'Optimizer rule.*' <<<"$out" | head -1 >&2
  exit 1
fi
# expected rows (a.c0, a.c1, b.c0): (1,x,1) (1,x,3)
rows="$(grep -E '^\| [0-9]' <<<"$out" | tr -d ' ')"
if [ "$rows" = "$(printf '|1|x|1|\n|1|x|3|')" ]; then
  exit 0
fi
echo "$out" >&2
exit 2
