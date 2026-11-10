#!/usr/bin/env bash
# Usage: repro.sh <path-to-brush> [<path-to-bash-5.3>]
# Exit 1 if brush's ${v:=word} / ${v=word} substitutes the word itself instead of the value
# the variable ends up with (declare -u/-l/-c transform the assigned value), 0 if not.
BRUSH=${1:?usage: repro.sh <path-to-brush> [<path-to-bash>]}
REF=${2:-}
BRUSH=$(realpath "$BRUSH"); [ -n "$REF" ] && REF=$(realpath "$REF")
SCRIPT='declare -u u; echo "${u:=abc} $u"
declare -l l; echo "${l:=ABC} $l"
declare -c c; echo "${c:=abc} $c"
declare -u e; echo "${e=xyz} $e"
declare -u -a arr; echo "${arr[1]:=q} ${arr[1]}"'
EXPECTED='ABC ABC
abc abc
Abc Abc
XYZ XYZ
Q Q'
run() { env -i LC_ALL=C PATH=/usr/bin:/bin "$@" -c "$SCRIPT"; }
actual=$(cd "$(mktemp -d)" && run "$BRUSH" --norc --noprofile --no-config)
echo "--- brush:"; printf '%s\n' "$actual"
if [ -n "$REF" ]; then echo "--- bash:"; (cd "$(mktemp -d)" && run "$REF" --norc --noprofile); fi
echo "--- expected (bash 5.3):"; printf '%s\n' "$EXPECTED"
[ "$actual" = "$EXPECTED" ] && { echo "OK: not reproduced"; exit 0; }
echo "BUG: reproduced"; exit 1
