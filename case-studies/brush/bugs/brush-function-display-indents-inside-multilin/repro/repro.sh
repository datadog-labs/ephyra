#!/usr/bin/env bash
# Usage: repro.sh <path-to-brush> [<path-to-bash>]
# Prints a function with a multi-line string, reads the output back with eval, and runs it.
# Exit 1 if brush adds indentation inside the string (bug present), 0 if not.
BRUSH=${1:?usage: repro.sh <path-to-brush> [<path-to-bash>]}
BASH_REF=${2:-bash}
script='f() { echo "a
b"; }
eval "$(declare -f f)"; f'
want=$(printf 'a\nb')
got=$("$BRUSH" --norc --noprofile -c "$script" 2>&1)
ref=$("$BASH_REF" --norc --noprofile -c "$script" 2>&1)
printf '%s\n' "bash : $(printf %q "$ref")" "brush: $(printf %q "$got")"
[ "$got" = "$want" ] && exit 0 || exit 1
