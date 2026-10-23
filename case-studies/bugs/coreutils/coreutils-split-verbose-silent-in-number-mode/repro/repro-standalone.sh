#!/bin/bash
# split --verbose prints nothing in -n/--number mode.
# Usage: repro-standalone.sh [path-to-split]   (default: split on PATH; for the
# multicall binary, symlink it as "split" or pass "coreutils split" via SPLIT env)
# Exit 1 = bug reproduces, 0 = does not.
SPLIT=${1:-split}
T=$(mktemp -d) && cd "$T" || exit 2
fail=0
for n in 2 l/2 r/2; do
  out=$(printf 'a\nb\nc\nd\n' | LC_ALL=C $SPLIT -n "$n" --verbose)
  want=$'creating file \'xaa\'\ncreating file \'xab\''
  if [ "$out" = "$want" ]; then echo "ok    -n $n"; else echo "FAIL  -n $n: got [${out}] want [${want//$'\n'/ | }]"; fail=1; fi
  rm -f x??
done
cd / && rm -rf "$T"
exit $fail
