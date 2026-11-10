#!/usr/bin/env bash
# sort: a key's own `r` is ignored when the key is the whole line (-k1r).
# Usage: repro.sh <dir-containing-sort-or-coreutils>
#   exit 1 = bug reproduces, 0 = does not, 2 = cannot run
D=${1:?usage: repro.sh <bin-dir>}
if [ -x "$D/sort" ]; then SORT=("$D/sort")
elif [ -x "$D/coreutils" ]; then SORT=("$D/coreutils" sort)
else echo "no sort or coreutils in $D" >&2; exit 2; fi
export LC_ALL=C
fail=0
check() { # expected-output args...
  local want=$1; shift
  local got
  got=$(printf 'a\n\nb\n' | "${SORT[@]}" "$@" | od -An -c | tr -s ' ')
  if [ "$got" = "$want" ]; then echo "ok   sort $*"; else
    echo "FAIL sort $*: got [$got], want [$want]"; fail=1; fi
}
# Descending: "b", "a", "" (what GNU coreutils 9.12 prints)
DESC=$(printf 'b\na\n\n' | od -An -c | tr -s ' ')
check "$DESC" -k1r
check "$DESC" -k1.1r
check "$DESC" -f -k1fr
check "$DESC" -k1,1r      # control: takes the per-key path, already correct
exit $fail
