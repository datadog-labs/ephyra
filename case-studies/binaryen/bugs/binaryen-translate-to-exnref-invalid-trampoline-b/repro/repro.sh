#!/bin/bash
# Usage: repro.sh <binaryen-build-dir>   (dir containing bin/wasm-opt)
# Exit 1: bug reproduces, 0: does not, 2: undecided.
B=$(realpath "$1")
cd "$(dirname "$0")"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
"$B/bin/wasm-opt" --version
# The input itself is valid:
"$B/bin/wasm-opt" poc.wat -all -o "$T/before.wasm" 2>/dev/null || { echo "input does not validate?"; exit 2; }
# The pass alone produces invalid IR:
"$B/bin/wasm-opt" poc.wat -all --translate-to-exnref -o "$T/after.wasm" > "$T/err" 2>&1
rc=$?
head -12 "$T/err"
[ $rc = 0 ] && { echo "OK: not reproduced"; exit 0; }
grep -q 'block with value must not have last element that is none' "$T/err" && { echo "REPRODUCED"; exit 1; }
exit 2
