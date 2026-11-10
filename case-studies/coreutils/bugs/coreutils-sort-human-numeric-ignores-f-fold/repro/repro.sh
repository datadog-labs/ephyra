#!/usr/bin/env bash
# sort: -f (--ignore-case) is ignored by -h (--human-numeric-sort); a lowercase SI
# suffix other than k ("1e", "2m", "3g") is not folded to its uppercase suffix.
# Usage: repro.sh <dir-containing-sort-or-coreutils>
#   exit 1 = bug reproduces, 0 = does not, 2 = cannot run
D=${1:?usage: repro.sh <bin-dir>}
if [ -x "$D/sort" ]; then SORT=("$D/sort")
elif [ -x "$D/coreutils" ]; then SORT=("$D/coreutils" sort)
else echo "no sort or coreutils in $D" >&2; exit 2; fi
export LC_ALL=C
fail=0
check() { # input expected args...
  local in=$1 want=$2; shift 2
  local got
  got=$(printf "$in" | "${SORT[@]}" "$@" | od -An -c | tr -s ' ')
  want=$(printf "$want" | od -An -c | tr -s ' ')
  if [ "$got" = "$want" ]; then echo "ok   sort $*"; else
    echo "FAIL sort $*: got [$got], want [$want]"; fail=1; fi
}
# Expected outputs are what GNU coreutils 9.12 prints.
check '1e\n1K\n'          '1K\n1e\n'          -h -f
check '1e\n2m\n1K\n3g\n5\n' '5\n1K\n2m\n3g\n1e\n' -h -f
check 'a 1e\nb 1K\n'      'b 1K\na 1e\n'      -k2,2hf
check '1e\n1K\n'          '1e\n1K\n'          -h    # control: without -f, "1e" has no suffix
exit $fail
