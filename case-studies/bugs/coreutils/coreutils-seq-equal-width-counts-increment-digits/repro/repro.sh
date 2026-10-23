#!/usr/bin/env bash
# seq -w pads to the width of the increment, which is never printed.
# Usage: repro.sh <path-to-seq>   e.g. repro.sh ./target/debug/seq
# Exit 1 if any case differs from the expected (GNU coreutils 9.12) output, 0 if all match.
SEQ=${1:?usage: repro.sh <path-to-uutils-seq>}
export LC_ALL=C
fail=0
check() { # $1 = args, $2 = expected stdout (lines joined by spaces)
  local out
  # shellcheck disable=SC2086
  out=$("$SEQ" -w $1 | tr '\n' ' ' | sed 's/ $//')
  if [ "$out" = "$2" ]; then echo "ok    seq -w $1  ->  $out"; else
    echo "FAIL  seq -w $1"; echo "      expected: $2"; echo "      actual:   $out"; fail=1; fi
}

check "1 -1 1"     "1"          # negative step: its '-' sign is counted as a digit
check "5 -1 3"     "5 4 3"
check "-3 -1 -5"   "-3 -4 -5"
check "1 007 8"    "1 8"        # zero-padded step widens the output
check "10 -1 8"    "10 09 08"   # control: first is already wider, both agree
exit $fail
