#!/usr/bin/env bash
# split -a 0 with -n N: GNU documents that -a 0 is "the same as if -a was not
# specified", so the suffix length starts at 2 (xaa, xab). uutils writes xa, xb.
#   repro.sh [path-to-split]   (default: split on PATH; for a multicall build pass
#                               a symlink named split -> coreutils, or "coreutils split")
# exit 1 = bug reproduces, 0 = behaves as documented, 2 = could not run
SPLIT=${1:-split}
T=$(mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT
cd "$T" || exit 2

check() { # $1 = expected file list, rest = split args
  local want=$1; shift
  rm -f x*
  $SPLIT "$@" < /dev/null || { echo "split $* failed" >&2; exit 2; }
  local got; got=$(ls x* 2>/dev/null | tr '\n' ' ')
  printf '%-28s expected: %-14s actual: %s\n' "split $*" "$want" "$got"
  [ "$got" = "$want" ]
}

bad=0
check 'xaa xab '   -n 2        || bad=1   # control: no -a
check 'xaa xab '   -n 2 -a 0   || bad=1   # -a 0 must behave like no -a
check 'x00 x01 '   -d -n 2 -a 0 || bad=1   # same with numeric suffixes
[ $bad = 1 ] && { echo "BUG: -a 0 did not behave as if -a was not specified"; exit 1; }
echo "OK"; exit 0
