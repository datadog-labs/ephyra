#!/usr/bin/env bash
# Usage: repro.sh [path-to-brush] [path-to-bash]
# Expected output is that of GNU bash 5.3 (globskipdots on by default, so . and .. are not listed).
# The first line (plain .*) is a control that already works.
# Exit 1 = bug reproduces (brush output differs from expected), 0 = fixed.
BRUSH=${1:-brush}
BASH_REF=${2:-}
d=$(mktemp -d) || exit 2
trap 'rm -rf "$d"' EXIT
cd "$d" || exit 2
mkdir sub
: > .h
: > sub/.s
script='shopt -u dotglob
empty=
echo .*
echo "".*
echo '"''"'.*
echo $empty"".*
echo $(true)'"''"'.*
echo sub/"".*'
expected='.h
.h
.h
.h
.h
sub/.s'
if [ -n "$BASH_REF" ]; then
  echo "--- bash ($BASH_REF):"; env -i LC_ALL=C PATH=/usr/bin:/bin "$BASH_REF" --norc --noprofile -c "$script"
fi
actual=$(env -i LC_ALL=C PATH=/usr/bin:/bin "$BRUSH" --norc --noprofile -c "$script" 2>&1)
echo "--- brush ($BRUSH):"; printf '%s\n' "$actual"
if [ "$actual" = "$expected" ]; then echo "OK: matches expected"; exit 0; fi
echo "BUG: expected:"; printf '%s\n' "$expected"; exit 1
