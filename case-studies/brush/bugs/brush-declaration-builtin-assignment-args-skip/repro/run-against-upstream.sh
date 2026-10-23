#!/usr/bin/env bash
# Usage: run-against-upstream.sh /path/to/brush [/path/to/bash-5.3]
# Prints what brush and (optionally) bash 5.3 do with script.sh plus a few extra declare cases.
cd "$(dirname "$0")" || exit 2
BRUSH=${1:?usage: $0 /path/to/brush [/path/to/bash]}
BASH53=${2:-}
extra='declare v={c,d}; declare -p v
declare a[{1,2}]=x; declare -p a
export e={c,d}; declare -p e'
for sh in "$BRUSH" $BASH53; do
  echo "=== $sh"
  "$sh" --norc --noprofile script.sh 2>&1
  "$sh" --norc --noprofile -c "$extra" 2>&1
done
