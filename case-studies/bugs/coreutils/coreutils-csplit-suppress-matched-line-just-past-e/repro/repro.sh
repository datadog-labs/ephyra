#!/usr/bin/env bash
# csplit --suppress-matched with line number N = (number of input lines + 1).
#   repro.sh <dir-with-uutils-csplit-or-coreutils>
# The argument is a directory holding either a `csplit` binary or the multicall
# `coreutils` binary (e.g. target/debug or target/release of an upstream build).
# Exit 1 = bug reproduces, 0 = fixed (behaves like GNU 9.12), 2 = cannot run.
set -u
D=$(cd "${1:?usage: repro.sh <dir with csplit or coreutils>}" && pwd) || exit 2
if [ -x "$D/csplit" ]; then CSPLIT=("$D/csplit")
elif [ -x "$D/coreutils" ]; then CSPLIT=("$D/coreutils" csplit)
else echo "no csplit or coreutils in $D" >&2; exit 2; fi

T=$(mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT
cd "$T" || exit 2

echo "\$ printf 'a\\n' | csplit --suppress-matched - 2"
printf 'a\n' | LC_ALL=C "${CSPLIT[@]}" --suppress-matched - 2
rc=$?
echo "exit=$rc"
for f in xx*; do [ -e "$f" ] && printf '%s: %q\n' "$f" "$(cat "$f")"; done

# GNU coreutils 9.12 prints "2" and "0", exits 0, and leaves xx00='a' and an empty xx01.
if [ "$rc" = 0 ] && [ -f xx00 ] && [ "$(cat xx00)" = a ] && [ -f xx01 ] && [ ! -s xx01 ]; then
  echo "OK: matches GNU 9.12"; exit 0
fi
echo "BUG: GNU 9.12 exits 0 and writes xx00='a' and an empty xx01"; exit 1
