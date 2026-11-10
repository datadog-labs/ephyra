#!/bin/sh
# Usage: repro.sh <ty-binary-or-build-dir>
# Exit 1: ty reports the false invalid-argument-type on repro.py (bug present).
# Exit 0: ty accepts repro.py. Exit 2: could not decide.
here=$(cd "$(dirname "$0")" && pwd)
ty=${1:-ty}
[ -d "$ty" ] && ty="$ty/bin/ty"
case "$ty" in */*) ty=$(cd "$(dirname "$ty")" && pwd)/$(basename "$ty") || exit 2;; esac
d=$(mktemp -d) || exit 2
trap 'rm -rf "$d"' EXIT
cp "$here/repro.py" "$d/repro.py"
cd "$d" || exit 2
out=$("$ty" check --python-version 3.13 --output-format concise --no-progress repro.py 2>&1)
rc=$?
printf '%s\n' "$out"
case "$out" in *panic*) exit 2;; esac
if printf '%s\n' "$out" | grep -q 'repro.py:7:.*invalid-argument-type'; then exit 1; fi
[ $rc -eq 0 ] && exit 0
exit 2
