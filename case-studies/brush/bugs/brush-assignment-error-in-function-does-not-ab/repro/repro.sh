#!/usr/bin/env bash
# An assignment error (out-of-range negative subscript) inside a function body must
# abort the whole command line that called the function, as in bash 5.3, which prints
# only "after". brush only fails the function call and goes on to print "rc=1".
# Exit 1 = reproduces, 0 = fixed, 2 = cannot decide.
cd "$(dirname "$0")" || exit 2
B=${1:?usage: repro.sh <build-dir>}
[ -x "$B/brush" ] || exit 2
out=$(cd "$(mktemp -d)" && env -i LC_ALL=C PATH=/usr/bin:/bin "$B/brush" --norc --noprofile "$OLDPWD/script.sh" 2>/dev/null)
case $out in
  'after') exit 0 ;;
  $'rc=1\nafter') exit 1 ;;
  *) echo "unexpected: $out" >&2; exit 2 ;;
esac
