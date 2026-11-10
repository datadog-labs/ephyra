#!/usr/bin/env bash
# Usage: repro.sh <path-to-brush-binary-or-dir-containing-brush>
# A variable assignment / arithmetic error on one line of a multi-line $( ... ) should
# abort the whole command substitution (bash 5.3: v=<> status=1 for each case).
# brush continues with the next line inside the substitution.
# Exit 1 = bug reproduces, 0 = behaves like bash, 2 = cannot decide.
cd "$(dirname "$0")" || exit 2
B=${1:?usage: repro.sh <brush binary or build dir>}
[ -d "$B" ] && B="$B/brush"
[ -x "$B" ] || { echo "no brush at $B" >&2; exit 2; }
expected=$'v=<> status=1\nv=<> status=1\nv=<> status=1'
out=$(env -i LC_ALL=C PATH=/usr/bin:/bin "$B" --norc --noprofile ./script.sh 2>/dev/null)
printf '%s\n' "$out"
[ "$out" = "$expected" ] && exit 0
case $out in *after*) exit 1 ;; esac
exit 2
