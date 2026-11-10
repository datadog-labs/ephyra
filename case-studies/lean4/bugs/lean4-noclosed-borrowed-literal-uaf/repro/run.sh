#!/bin/sh
# run.sh <toolchain-bin>: builds and runs both repros with and without closed term extraction.
# Prints the four outputs; the extract_closed=false builds are the buggy ones.
T=$(cd "$1" && pwd)
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
cd "$tmp" || exit 2
for f in String Nat; do
  cp "$here/$f.lean" .
  for o in true false; do
    PATH="$T:$PATH" "$T/lean" -Dcompiler.extract_closed=$o -c "$f-$o.c" "$f.lean" || exit 2
    PATH="$T:$PATH" "$T/leanc" -O3 -o "$f-$o" "$f-$o.c" || exit 2
    printf '%s extract_closed=%s: %s\n' "$f" "$o" "$(./"$f-$o" < /dev/null 2>&1 | head -1)"
  done
done
