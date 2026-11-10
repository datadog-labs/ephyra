#!/usr/bin/env bash
# brush ignores patsub_replacement: an unquoted & in ${var/pattern/string}
# should stand for the matched text (bash 5.2+, option on by default).
# Usage: repro.sh [path-to-brush] [path-to-bash]
# Exit 1 = bug reproduces, 0 = brush matches bash, 2 = cannot decide.
BRUSH=${1:-brush}
BASH_REF=${2:-bash}
script='v=abc; r="<&>"; echo "${v/b/<&>}" "${v/b/$r}"'
want=$("$BASH_REF" --norc --noprofile -c "$script" 2>&1)
got=$("$BRUSH" --norc --noprofile -c "$script" 2>&1)
echo "bash:  $want"
echo "brush: $got"
[ "$want" = 'a<b>c a<b>c' ] || { echo "reference bash did not give the expected output" >&2; exit 2; }
[ "$got" = "$want" ] && exit 0
exit 1
