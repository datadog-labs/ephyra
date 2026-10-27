#!/bin/sh
# Usage: repro.sh <ty-binary | build-dir containing bin/ty>
# Exit 1: ty reports invalid-assignment on repro.py (bug present).
# Exit 0: no diagnostic on repro.py (bug absent). Exit 2: cannot decide.
here=$(cd "$(dirname "$0")" && pwd)
t=${1:-ty}
[ -d "$t" ] && t="$t/bin/ty"
case "$t" in */*) t=$(cd "$(dirname "$t")" && pwd)/$(basename "$t") || exit 2;; esac
d=$(mktemp -d) || exit 2
trap 'rm -rf "$d"' EXIT
cp "$here/repro.py" "$here/single.py" "$d/" && cd "$d" || exit 2
# Control: the same lambda against a single Callable must pass.
"$t" check --python-version 3.13 --output-format concise --no-progress single.py >/dev/null 2>&1 || exit 2
out=$("$t" check --python-version 3.13 --output-format concise --no-progress repro.py 2>&1)
rc=$?
echo "$out"
case "$out" in *panic*) exit 2;; esac
echo "$out" | grep -q 'repro.py:3:.*invalid-assignment' && exit 1
[ $rc -eq 0 ] && exit 0
exit 2
