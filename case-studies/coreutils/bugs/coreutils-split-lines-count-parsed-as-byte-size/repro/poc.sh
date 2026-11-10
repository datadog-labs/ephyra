#!/usr/bin/env bash
# split -l takes a plain line count. uutils parses it like a byte SIZE:
# a leading 0 is read as octal, and units/hex are accepted.
# Usage: poc.sh [dir-holding-split]   (default: split on PATH)
# Exit 1 if the bug shows (uutils main), 0 if split behaves like GNU 9.12.
[ -n "$1" ] && PATH="$1:$PATH"
T=$(mktemp -d) && cd "$T" || exit 2
bug=0
seq 20 | split -l 010
echo "split -l 010 (20 lines):"; wc -l x*
# GNU 9.12: two files of 10 lines.  uutils: 8, 8, 4.
[ "$(ls x* | wc -l)" = 2 ] || bug=1
rm -f x*
for n in 1k 0x10; do
  if seq 20 | split -l "$n" 2>/dev/null; then
    echo "split -l $n: accepted (GNU 9.12: invalid number of lines)"; bug=1
  else
    echo "split -l $n: rejected"
  fi
  rm -f x*
done
cd / && rm -rf "$T"
exit $bug
