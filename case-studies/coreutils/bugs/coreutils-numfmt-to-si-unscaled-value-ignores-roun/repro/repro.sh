#!/usr/bin/env bash
# numfmt --to=si/iec: a value that stays without a suffix ignores --round
# (12.1 -> 12 under the default from-zero), and values just below 1000/1024
# get a suffix even when the rounding method keeps them below the base
# (999.9 --round=down -> 0.9k).
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
    printf 'ok    numfmt %-42s -> %s\n' "$*" "$got"
  else
    printf 'FAIL  numfmt %-42s -> %s (expected %s)\n' "$*" "$got" "$want"
    fail=1
  fi
}

check 13     --to=si 12.1
check 13     --to=si --round=up 12.1
check -13    --to=si -- -12.1
check 13     --to=si 12.5
check 13     --to=iec 12.5
check 999    --to=si --round=down 999.9
check 999.0  --to=si --round=down --format=%.1f 999.9
check 999    --to=si --round=towards-zero 999.4
# controls: these already match GNU
check 1.0k   --to=si 999.4
check 1      --to=si 1.25
check 10     --to=si 9.96

[ $fail = 0 ] && { echo "all cases match GNU 9.12"; exit 0; }
exit 1
