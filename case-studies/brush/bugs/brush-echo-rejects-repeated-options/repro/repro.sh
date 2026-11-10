#!/usr/bin/env bash
# Usage: repro.sh <path-to-brush> [path-to-bash]
# Runs poc.sh with brush and bash and prints both outputs.
# Exit 1 if brush differs from the expected bash output, 0 if it matches.
cd "$(dirname "$0")" || exit 2
BRUSH=${1:?usage: repro.sh <path-to-brush> [path-to-bash]}
BASH_REF=${2:-bash}
expected=$(printf 'hi rc=0\na\\tb')
actual=$("$BRUSH" --norc --noprofile poc.sh 2>&1)
echo "--- bash:";  "$BASH_REF" --norc --noprofile poc.sh 2>&1
echo "--- brush:"; printf '%s\n' "$actual"
if [ "$actual" = "$expected" ]; then echo "OK: brush matches bash"; exit 0; fi
echo "BUG: brush output differs"; exit 1
