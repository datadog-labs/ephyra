#!/usr/bin/env bash
# cat -s drops a single blank line when the line before it has no newline
# at the end of the previous file.
#
# usage: repro.sh <cat-command...>
#   repro.sh /path/to/target/release/coreutils cat   (uutils multicall)
#   repro.sh /path/to/gnu/bin/cat                     (GNU, for comparison)
# Exit 1 if the output differs from GNU coreutils 9.12, 0 if it matches.
set -u
[ $# -ge 1 ] || { echo "usage: $0 <cat-command...>" >&2; exit 2; }
CAT=("$@")
export LC_ALL=C
d=$(mktemp -d) || exit 2
trap 'rm -rf "$d"' EXIT

printf 'a' > "$d/f1"         # no trailing newline
printf '\n\nb\n' > "$d/f2"   # ends line "a", then one blank line, then "b"

want=$'a\n\nb\n'             # GNU 9.12 output
got=$("${CAT[@]}" -s "$d/f1" "$d/f2"; echo x); got=${got%x}

if [ "$got" = "$want" ]; then
  echo "ok:   cat -s f1 f2 -> $(printf %q "$got")"
  exit 0
fi
echo "DIFF: cat -s f1 f2"
echo "  expected: $(printf %q "$want")"
echo "  actual:   $(printf %q "$got")"
exit 1
