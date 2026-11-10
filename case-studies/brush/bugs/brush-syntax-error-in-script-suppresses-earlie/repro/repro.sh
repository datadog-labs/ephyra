#!/usr/bin/env bash
# Usage: repro.sh <path-to-brush> [path-to-bash]
# A script whose line 2 is a syntax error. bash runs line 1 first; brush runs nothing.
# Exit 1 = bug reproduces (brush does not print "before"), 0 = it does not, 2 = usage error.
BRUSH=${1:?usage: repro.sh <path-to-brush> [path-to-bash]}
BASH_REF=${2:-bash}
tmp=$(mktemp -d) || exit 2
trap 'rm -rf "$tmp"' EXIT
printf 'echo before\nfi\n' > "$tmp/t.sh"

for sh in "$BASH_REF" "$BRUSH"; do
  echo "== $sh (script file)"; "$sh" "$tmp/t.sh"; echo "exit=$?"
  echo "== $sh (-c)";          "$sh" -c $'echo before\nfi'; echo "exit=$?"
done

out=$("$BRUSH" "$tmp/t.sh" 2>/dev/null)
[ "$out" = before ] && exit 0 || exit 1
