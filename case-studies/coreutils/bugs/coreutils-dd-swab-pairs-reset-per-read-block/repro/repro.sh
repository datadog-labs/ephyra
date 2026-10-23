#!/usr/bin/env bash
# dd conv=swab must pair bytes across the whole input, not inside each read block.
# usage: repro.sh <dir-with-uutils-dd> [<dir-with-gnu-dd>]
# exit 1 = bug reproduces, 0 = does not, 2 = cannot decide
U=${1:?usage: repro.sh <uutils-bin-dir> [gnu-bin-dir]}
G=${2:-}
[ -x "$U/dd" ] || exit 2
run() { # $1 = bin dir
  printf ab | "$1/dd" conv=swab bs=1 status=none; echo
  printf abcdefg | "$1/dd" conv=swab bs=3 status=none; echo
}
echo "expected:"; printf 'ba\nbadcfeg\n'
if [ -n "$G" ]; then echo "gnu:"; run "$G"; fi
out=$(run "$U"); echo "uutils:"; echo "$out"
[ "$out" = $'ba\nbadcfeg' ] && exit 0
exit 1
