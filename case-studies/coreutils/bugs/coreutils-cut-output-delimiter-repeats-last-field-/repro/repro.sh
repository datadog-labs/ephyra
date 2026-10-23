#!/usr/bin/env bash
# cut --output-delimiter prints the last field twice when the last line has no
# trailing newline and a later range starts past the last field.
#   repro.sh <path-to-cut>   (uutils `cut`, or `coreutils` multicall: pass "coreutils cut")
# exit 1 = bug reproduces, 0 = output matches GNU, 2 = cannot run
CUT=${1:?usage: repro.sh <path-to-cut>}
fail=0
check() { # expected, input, args...
  local want=$1 in=$2; shift 2
  local got
  got=$(printf "$in" | LC_ALL=C $CUT "$@") || exit 2
  if [ "$got" = "$want" ]; then
    echo "ok   printf '$in' | cut $*  -> $got"
  else
    echo "FAIL printf '$in' | cut $*  -> $got (expected $want)"; fail=1
  fi
}
check 'a:b'   'a\tb'   --output-delimiter=: -f 1-2,3
check 'a:b'   'a,b'    -d, --output-delimiter=: -f 1-2,3
check 'b:c'   'a,b,c'  -d, --output-delimiter=: -f 2-3,4-
exit $fail
