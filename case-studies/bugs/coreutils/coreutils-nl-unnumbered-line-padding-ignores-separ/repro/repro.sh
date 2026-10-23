#!/usr/bin/env bash
# nl pads unnumbered lines with (width + 1) spaces instead of (width + separator length).
# Usage: repro.sh [path-to-nl]   (default: nl on PATH; for uutils pass e.g. target/release/nl,
#        or "target/release/coreutils nl" via NL env: NL="target/release/coreutils nl" repro.sh)
# Exit 1 = bug reproduces, 0 = output matches GNU coreutils 9.12, 2 = usage error.
set -u
if [ -n "${NL:-}" ]; then read -r -a CMD <<<"$NL"; else CMD=("${1:-nl}"); fi
export LC_ALL=C
fail=0
check() { # $1 = description, $2 = expected stdout, rest = nl args; input "a\n\nb\n"
  local desc=$1 want=$2; shift 2
  local got
  got=$(printf 'a\n\nb\n' | "${CMD[@]}" "$@"; echo x) || return 2
  got=${got%x}
  if [ "$got" = "$want" ]; then echo "ok   $desc"
  else echo "FAIL $desc"; printf '  expected: %q\n  actual:   %q\n' "$want" "$got"; fail=1; fi
}
# Expected values are GNU coreutils 9.12 output.
check "-s ':-:'  (3-byte separator)" $'     1:-:a\n         \n     2:-:b\n' -s ':-:'
check "-s '  '   (2-byte separator)" $'     1  a\n        \n     2  b\n'   -s '  '
check "-s ''     (empty separator)"  $'     1a\n      \n     2b\n'        -s ''
check "-s ':-:' -w 3"                $'  1:-:a\n      \n  2:-:b\n'       -s ':-:' -w 3
check "default TAB separator (control)" $'     1\ta\n       \n     2\tb\n'
exit $fail
