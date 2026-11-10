#!/usr/bin/env bash
# Standalone repro: a negative subscript on a sparse indexed array is resolved
# relative to the element count instead of (max index + 1).
#
# usage: repro.sh [path-to-brush] [path-to-bash]
# Exit 1 = bug reproduces, 0 = brush matches the expected output, 2 = setup error.
BRUSH=${1:-brush}
BASH_REF=${2:-}
command -v "$BRUSH" >/dev/null 2>&1 || { echo "brush not found: $BRUSH" >&2; exit 2; }

script='b=(a b c d)
b[5]=q
echo "<${b[-2]}> <${b[-1]}>"
b[-2]=Z
declare -p b
unset '\''b[-3]'\''
declare -p b'

# Expected output, from bash 5.3.
expected='<> <q>
declare -a b=([0]="a" [1]="b" [2]="c" [3]="d" [4]="Z" [5]="q")
declare -a b=([0]="a" [1]="b" [2]="c" [4]="Z" [5]="q")'

if [ -n "$BASH_REF" ]; then
  expected=$("$BASH_REF" --norc --noprofile -c "$script")
fi

actual=$("$BRUSH" --norc --noprofile -c "$script" 2>&1)

echo "--- expected"; printf '%s\n' "$expected"
echo "--- actual ($BRUSH)"; printf '%s\n' "$actual"
if [ "$actual" = "$expected" ]; then
  echo "OK: not reproduced"; exit 0
fi
echo "BUG: negative subscript resolved against element count"; exit 1
