#!/usr/bin/env bash
# chmod: a symbolic mode that starts with '-' followed by '=', '+' or ',' is rejected as an option.
# usage: repro.sh <dir-with-chmod> [<dir-with-gnu-chmod>]
# exit 1 = bug reproduces with the first chmod, 0 = it does not.
set -u
UU=${1:?usage: repro.sh <dir-with-chmod> [<dir-with-gnu-chmod>]}
GNU=${2:-}
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cd "$T" || exit 2
umask 022
bad=0
for impl in ${GNU:+gnu} uu; do
  [ "$impl" = gnu ] && dir=$GNU || dir=$UU
  for mode in -=r -+x -,u+x; do
    touch f; command chmod 644 f
    PATH=$dir chmod "$mode" f 2>err; rc=$?
    echo "$impl: chmod $mode f -> exit $rc, mode $(stat -c %A f)${rc:+ $(head -1 err)}"
    [ "$impl" = uu ] && [ "$rc" != 0 ] && bad=1
  done
done
exit $bad
