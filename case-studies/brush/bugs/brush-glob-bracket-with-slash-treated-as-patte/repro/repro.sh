#!/usr/bin/env bash
# Usage: repro.sh /path/to/brush
# bash 5.3 prints "< [s/x] >": a bracket expression that contains '/' makes '[' an
# ordinary character, so the word is not a pattern and nullglob does not remove it.
# Exit 1 = brush removes the word (bug reproduces), 0 = it keeps it, 2 = cannot decide.
BRUSH=${1:?usage: repro.sh /path/to/brush}
case "$BRUSH" in /*) ;; *) BRUSH=$PWD/$BRUSH ;; esac
[ -x "$BRUSH" ] || { echo "not executable: $BRUSH" >&2; exit 2; }
T=$(mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT
out=$(cd "$T" && env -i LC_ALL=C PATH=/usr/bin:/bin "$BRUSH" --norc --noprofile -c 'shopt -s nullglob; echo "<" [s/x] ">"' 2>/dev/null)
echo "brush printed: $out   (bash 5.3 prints: < [s/x] >)"
case "$out" in
  "< [s/x] >") exit 0 ;;
  "< >") exit 1 ;;
  *) exit 2 ;;
esac
