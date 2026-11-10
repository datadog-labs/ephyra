#!/usr/bin/env bash
# Usage: repro.sh <path-to-brush-binary-or-build-dir>
# Runs poc.sh with brush and compares with the output bash 5.3 gives.
# Exit 1 = bug reproduces, 0 = brush matches bash, 2 = cannot run.
B=${1:?usage: repro.sh <brush-binary|build-dir>}
B=$(realpath "$B") || exit 2
cd "$(dirname "$0")" || exit 2
[ -d "$B" ] && { for c in "$B/brush" "$B/debug/brush" "$B/release/brush"; do [ -x "$c" ] && { B=$c; break; }; done; }
[ -x "$B" ] || exit 2
expected=$(printf '   a 001   b\nstatus=0 v=x\003y\nstatus=0 v=x\004y')
actual=$(env -i LC_ALL=C PATH=/usr/bin:/bin "$B" --norc --noprofile poc.sh 2>&1) || true
[ "$actual" = "$expected" ] && exit 0
printf 'expected (bash 5.3):\n%s\nactual:\n%s\n' "$expected" "$actual" | cat -v
exit 1
