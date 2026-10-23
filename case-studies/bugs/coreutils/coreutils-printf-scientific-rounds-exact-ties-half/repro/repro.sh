#!/usr/bin/env bash
# printf %e/%g rounds an exact halfway value up instead of to even.
# Usage: repro.sh <dir holding the printf binary to test>
#   exit 1 = bug present, 0 = output matches GNU 9.12, 2 = usage error
D=${1:?usage: repro.sh <bin-dir>}
[ -x "$D/printf" ] || exit 2
out=$(env -i LC_ALL=C "$D/printf" '%.1e %.0e %.1g %.0f\n' 0.125 2.5 0.25 2.5)
want='1.2e-01 2e+00 0.2 2'   # GNU coreutils 9.12 output
echo "got:  $out"
echo "want: $want"
[ "$out" = "$want" ] && exit 0
exit 1
