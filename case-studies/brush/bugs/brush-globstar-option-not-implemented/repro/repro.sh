#!/usr/bin/env bash
# brush: `shopt -s globstar` is accepted, but `**` does not recurse into subdirectories.
# Usage: repro.sh <path-to-brush> [<path-to-bash>]
# Exit 1 = bug reproduces, 0 = brush matches the expected output, 2 = cannot decide.
BRUSH=${1:?usage: repro.sh <path-to-brush> [<path-to-bash>]}
BASH_REF=${2:-}
[ -x "$BRUSH" ] || exit 2

script='mkdir sub; touch top.txt sub/deep.txt; shopt -s globstar; echo **; echo **/*.txt'
expected='sub sub/deep.txt top.txt
sub/deep.txt top.txt'

run() {
  local dir out
  dir=$(mktemp -d) || exit 2
  out=$(cd "$dir" && LC_ALL=C "$1" --norc --noprofile -c "$script" 2>&1)
  rm -rf "$dir"
  printf '%s\n' "$out"
}

if [ -n "$BASH_REF" ]; then
  echo "--- bash ($BASH_REF)"
  run "$BASH_REF"
fi
echo "--- brush ($BRUSH)"
actual=$(run "$BRUSH")
printf '%s\n' "$actual"

[ -n "$actual" ] || exit 2
[ "$actual" = "$expected" ] && exit 0
exit 1
