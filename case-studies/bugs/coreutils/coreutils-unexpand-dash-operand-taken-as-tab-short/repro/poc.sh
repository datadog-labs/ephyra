#!/usr/bin/env bash
# PoC: a bare "-" (standard input) operand is taken for an obsolete "-N" tab option.
# Usage: poc.sh <dir holding unexpand and expand>   (e.g. a dir of symlinks to the uutils multicall binary)
# Exit 1 if the bug shows, 0 if output matches GNU coreutils 9.12, 2 on usage error.
B=${1:?usage: poc.sh <bin-dir>}
[ -x "$B/unexpand" ] && [ -x "$B/expand" ] || exit 2
T=$(mktemp -d) || exit 2; trap 'rm -rf "$T"' EXIT
cd "$T" && printf 'file\n' > f
bad=0
# 1. -t implies -a, so the blanks after "a" become a tab (GNU: "a\tb").
out=$(printf 'a  b\n' | "$B/unexpand" -t 3 - | sed -n l)
echo "unexpand -t 3 -   :$out"
[ "$out" = 'a\tb$' ] || bad=1
# 2. "-" before a file must read stdin first (GNU: "stdin" then "file").
for u in unexpand expand; do
  out=$(printf 'stdin\n' | "$B/$u" - f | tr '\n' ' ')
  echo "$u - f :     $out"
  [ "$out" = "stdin file " ] || bad=1
done
exit $bad
