#!/usr/bin/env bash
# csplit: a /regex/ after a line-number pattern N does not test line N.
#   repro.sh [csplit-command...]   default: csplit on PATH. Example: ./repro.sh /path/to/coreutils csplit
# Exit 1 = bug reproduces, 0 = output matches GNU coreutils 9.12, 2 = could not run.
set -u
CSPLIT=("${@:-csplit}")
case ${CSPLIT[0]} in */*) CSPLIT[0]=$(cd "$(dirname "${CSPLIT[0]}")" && pwd)/$(basename "${CSPLIT[0]}") ;; esac
T=$(mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT
cd "$T" || exit 2
fail=0
check() { # $1 = expected stdout+status, $2 = stdin, rest = csplit args
  local want=$1 in=$2; shift 2
  rm -f xx*
  local got
  got=$(printf "$in" | LC_ALL=C "${CSPLIT[@]}" "$@" 2>&1; echo "exit=$?")
  if [ "$got" = "$want" ]; then echo "ok:   csplit $*"; else
    echo "FAIL: csplit $*"; echo "  expected: $(echo "$want" | tr '\n' ' ')"
    echo "  actual:   $(echo "$got" | tr '\n' ' ')"; fail=1; fi
}
# Line 2 ("1") matches /[0-9]/ and is the current line after pattern 2: expect an empty split.
check $'2\n0\n4\nexit=0' 'a\n1\n2\n' - 2 '/[0-9]/'
# Line 3 matches /3/; uutils reports "match not found".
check $'4\n0\n4\nexit=0' '1\n2\n3\n4\n' - 3 /3/
exit $fail
