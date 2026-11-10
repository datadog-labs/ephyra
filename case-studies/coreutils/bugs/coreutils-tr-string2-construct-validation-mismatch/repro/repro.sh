#!/usr/bin/env bash
# tr: string2 constructs are validated against the wrong mode.
#
# usage: repro.sh <tr-command...>
#   repro.sh /path/to/target/debug/coreutils tr     (uutils multicall)
#   repro.sh /path/to/gnu/bin/tr                     (GNU, for comparison)
# Exit 1 if any case differs from GNU coreutils 9.12 (the expected values below
# were recorded from GNU 9.12, LC_ALL=C), 0 if all match.
set -u
[ $# -ge 1 ] || { echo "usage: $0 <tr-command...>" >&2; exit 2; }
TR=("$@")
export LC_ALL=C

fail=0
# check NAME STDIN EXPECTED_EXIT EXPECTED_STDOUT -- ARGS...
check() {
  local name=$1 in=$2 want_rc=$3 want_out=$4; shift 5
  local out rc
  out=$(printf '%s\n' "$in" | "${TR[@]}" "$@" 2>/tmp/tr-repro-err.$$)
  rc=$?
  if [ "$rc" = "$want_rc" ] && [ "$out" = "$want_out" ]; then
    printf 'ok    %-22s tr %s\n' "$name" "$*"
  else
    fail=1
    printf 'DIFF  %-22s tr %s\n' "$name" "$*"
    printf '      expected: exit %s, stdout %q\n' "$want_rc" "$want_out"
    printf '      actual:   exit %s, stdout %q, stderr: %s\n' "$rc" "$out" "$(cat /tmp/tr-repro-err.$$)"
  fi
  rm -f /tmp/tr-repro-err.$$
}

# -d -s: "any character class can be used in string2"
check A-ds-lower         xaab 0 xb   -- -ds a '[:lower:]'
check B-ds-cntrl-punct   ab   0 ''   -- -d -s '[:print:]b-z' '[:cntrl:][:punct:]b-z'
# -d -s: [c*] is only allowed in string2 when translating
check C-ds-char-star     xaab 1 ''   -- -ds a '[b*]'
# translating: [=c=] is not allowed in string2
check D-equiv-translate  xaab 1 ''   -- a '[=b=]'
# translating: upper/lower in string2 must line up with upper/lower in string1
check E-xdigit-upper     xaab 1 ''   -- '[:xdigit:]' '[:upper:]'
# GNU accepts these (set1 order fixed under -c; class past the end of set1 is unused)
check F-complement-lower xaAb 0 babbj -- -c a 'b[:lower:]b'
check G-lower-past-end   xaAb 0 xaAb -- a 'ab[:lower:]'

exit $fail
