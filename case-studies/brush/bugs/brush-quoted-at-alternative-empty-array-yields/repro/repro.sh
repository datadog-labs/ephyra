#!/usr/bin/env bash
# Standalone repro: "${arr[@]+word}", "${arr[@]:+word}", "${@+word}", "${@:+word}"
# expand to one empty word (instead of zero words) when the array is empty/unset
# or there are no positional parameters.
#
# usage: repro.sh [path-to-brush] [path-to-bash]
# Exit 1 = bug reproduces, 0 = brush matches the expected output, 2 = setup error.
BRUSH=${1:-brush}
BASH_REF=${2:-}
command -v "$BRUSH" >/dev/null 2>&1 || { echo "brush not found: $BRUSH" >&2; exit 2; }

script='b=()
unset u
printf "<%s>" "${b[@]+a}" x; echo
printf "<%s>" "${b[@]:+a}" x; echo
printf "<%s>" "${u[@]+a}" x; echo
set --
printf "<%s>" "${@+a}" x; echo
printf "<%s>" "${@:+a}" x; echo'

# Expected output, from bash 5.3.20 (each quoted expansion vanishes like "${b[@]}").
expected='<x>
<x>
<x>
<x>
<x>'

if [ -n "$BASH_REF" ]; then
  expected=$("$BASH_REF" --norc --noprofile -c "$script")
fi

actual=$("$BRUSH" --norc --noprofile -c "$script" 2>&1)

echo "--- expected"; printf '%s\n' "$expected"
echo "--- actual ($BRUSH)"; printf '%s\n' "$actual"
if [ "$actual" = "$expected" ]; then
  echo "OK: not reproduced"; exit 0
fi
echo "BUG: quoted [@]/@ alternative produced an empty word"; exit 1
