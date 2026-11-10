#!/usr/bin/env bash
# numfmt prints a signed zero ("-0", "-0.00", "-0000") when rounding takes a
# small negative number to zero. GNU coreutils 9.12 prints an unsigned zero.
#
# usage: repro.sh <numfmt>     e.g. repro.sh ./target/debug/numfmt
#        or the multicall form:     repro.sh "target/debug/coreutils numfmt"
# Expected values are what GNU coreutils 9.12 prints (LC_ALL=C).
# Exit: 1 = bug reproduces, 0 = all cases match GNU, 2 = usage error.
set -u
[ $# -ge 1 ] || { echo "usage: $0 <numfmt command>" >&2; exit 2; }
read -r -a NUMFMT <<< "$1"
export LC_ALL=C
fail=0

check() { # expected stdout, args...
  local want=$1 got; shift
  got=$("${NUMFMT[@]}" "$@" 2>&1)
  if [ "$got" == "$want" ]; then
    printf 'ok    numfmt %-44s -> %s\n' "$*" "$got"
  else
    printf 'FAIL  numfmt %-44s -> %s (expected %s)\n' "$*" "$got" "$want"
    fail=1
  fi
}

check 0      --round=up --format=%.0f -- -0.3
check 0.0    --round=nearest --format=%.1f -- -0.04
check 0      --to-unit=10 --round=nearest -- -3
check 00000  --round=up --format=%05.0f -- -0.3
# controls: these already match GNU
check 0      -- -0
check -1     --format=%.0f -- -0.3
check -0     --to=si -- -0.4
exit $fail
