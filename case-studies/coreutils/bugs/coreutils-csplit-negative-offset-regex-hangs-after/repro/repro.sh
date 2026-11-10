#!/usr/bin/env bash
# csplit hangs on a /REGEX/-N pattern that follows a regex pattern and a line-number pattern.
# Usage: repro.sh [csplit command...]   e.g. repro.sh ./target/debug/coreutils csplit
# Exit 1 = hangs (bug reproduces), 0 = finishes with GNU 9.12's output, 2 = other result.
[ $# -gt 0 ] || set -- csplit
case $1 in */*) set -- "$(cd "$(dirname "$1")" && pwd)/$(basename "$1")" "${@:2}" ;; esac
dir=$(mktemp -d) || exit 2
cd "$dir" || exit 2
out=$(seq 3 | LC_ALL=C timeout -s KILL 10 "$@" - /2/ 2 /3/-1 2>&1)
rc=$?
cd / && rm -rf "$dir"
printf '%s\nexit=%s\n' "$out" "$rc"
case $rc in
  124|137) echo "csplit did not finish in 10 seconds: bug reproduces"; exit 1 ;;
esac
if [ "$rc" = 0 ] && [ "$out" = "$(printf '2\n0\n0\n4')" ]; then
  echo "same as GNU 9.12: bug does not reproduce"; exit 0
fi
echo "unexpected result"; exit 2
