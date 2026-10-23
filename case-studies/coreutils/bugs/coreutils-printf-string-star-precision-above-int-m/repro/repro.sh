#!/usr/bin/env bash
# printf '%.*s' accepts a '*' precision above INT_MAX (2147483647).
# Usage: repro.sh <dir-with-uutils-printf> [<dir-with-gnu-printf>]
#   e.g. repro.sh target/release /usr/bin
# Exit 1 = bug reproduces (uutils prints and exits 0), 0 = it does not, 2 = usage error.
U=${1:?usage: repro.sh <uutils-bin-dir> [<gnu-bin-dir>]}
G=${2:-}
[ -x "$U/printf" ] || { echo "no printf in $U" >&2; exit 2; }

show() { # $1 = label, $2 = bin dir
  echo "# $1:"
  env -i PATH="$2" LC_ALL=C printf '%.*sX\n' 2147483648 hello
  echo "exit=$?"
}

[ -n "$G" ] && show "GNU (expected: invalid precision error, exit 1, no output)" "$G"
show "uutils" "$U"

out=$(env -i PATH="$U" LC_ALL=C printf '%.*sX' 2147483648 hello 2>/dev/null)
rc=$?
if [ "$rc" = 0 ] || [ -n "$out" ]; then
  echo "REPRODUCES: uutils accepted the precision"; exit 1
fi
echo "does not reproduce"; exit 0
