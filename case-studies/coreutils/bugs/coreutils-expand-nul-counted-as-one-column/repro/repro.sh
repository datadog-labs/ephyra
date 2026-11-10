#!/usr/bin/env bash
# expand: NUL must take zero columns (as in GNU coreutils 9.12).
# usage: repro.sh <expand binary | coreutils multicall binary | dir containing expand>
# exit 1 = bug reproduces, 0 = output matches GNU 9.12, 2 = usage error
set -u
B=${1:?usage: repro.sh <expand | coreutils | bin-dir>}
if [ -d "$B" ]; then B=$B/expand; fi
[ -x "$B" ] || { echo "not executable: $B" >&2; exit 2; }
case $(basename "$B") in coreutils) CMD=("$B" expand) ;; *) CMD=("$B") ;; esac
export LC_ALL=C
fail=0
check() { # $1 = input (printf format), $2 = expected output (printf format), rest = options
  local in=$1 want=$2; shift 2
  local got exp
  got=$(printf "$in" | "${CMD[@]}" "$@" | od -An -c)
  exp=$(printf "$want" | od -An -c)
  if [ "$got" = "$exp" ]; then echo "ok:   expand $* <<< '$in'"
  else echo "FAIL: expand $* <<< '$in'"; echo "  expected:$exp"; echo "  actual:  $got"; fail=1; fi
}
# expected outputs recorded from GNU coreutils 9.12 (LC_ALL=C)
check '\0\t' '\0        '                       # NUL then TAB: 8 spaces
check '\0\ta\0b\tc\n' '\0        a\0b      c\n'
check 'ab\0\0cd\tx\n' 'ab\0\0cd    x\n'           # 4 visible chars -> 4 spaces
check '\0\t' '\0    ' -t4
exit $fail
