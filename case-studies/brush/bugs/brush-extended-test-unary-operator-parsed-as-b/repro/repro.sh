#!/usr/bin/env bash
# Usage: repro.sh <path-to-brush> [<path-to-bash-5.3>]
# Exit 1 if brush differs from bash on these [[ ]] cases, 0 if it matches.
# An unquoted unary operator in [[ ]] must take the next word as its operand.
BRUSH=${1:?usage: repro.sh <brush> [bash]}
BASH53=${2:-bash}
s1='[[ -z == ]]; echo "rc=$?"'     # valid: -z applied to the word "=="
s2='[[ -z == x ]]; echo "rc=$?"'   # syntax error: "x" is an extra word
run() { "$1" -c "$2" 2>/dev/null; echo "exit=$?"; }
fail=0
for s in "$s1" "$s2"; do
  b=$(run "$BASH53" "$s"); r=$(run "$BRUSH" "$s")
  printf '%s\n  # bash:  %s\n  # brush: %s\n' "$s" "$(echo $b)" "$(echo $r)"
  [ "$b" = "$r" ] || fail=1
done
exit $fail
