#!/usr/bin/env bash
# numfmt --invalid=fail|warn|ignore: one non-numeric field makes the whole
# line come out unconverted.
#
# usage: repro.sh <numfmt>     e.g. repro.sh ./numfmt   (a symlink to the uutils binary)
#        or the multicall form:     repro.sh "target/debug/coreutils numfmt"
# Expected values are what GNU coreutils 9.12 prints (LC_ALL=C).
# Exit: 1 = bug reproduces, 0 = all cases match GNU, 2 = usage error.
set -u
[ $# -ge 1 ] || { echo "usage: $0 <numfmt command>" >&2; exit 2; }
read -r -a NUMFMT <<< "$1"
export LC_ALL=C
fail=0

check() { # name, expected stdout (printf format), expected #diagnostics, stdin (printf format), args...
  local name=$1 want=$2 want_diag=$3 input=$4; shift 4
  local got diag
  got=$(printf "$input" | "${NUMFMT[@]}" "$@" 2>"$TMP/err"; echo x); got=${got%x}
  diag=$(grep -c 'invalid number' "$TMP/err")
  want=$(printf "$want"; echo x); want=${want%x}
  if [ "$got" == "$want" ] && [ "$diag" == "$want_diag" ]; then
    echo "ok    $name"
  else
    echo "FAIL  $name"
    printf '      expected stdout: %q  (%s diagnostics)\n' "$want" "$want_diag"
    printf '      actual   stdout: %q  (%s diagnostics)\n' "$got" "$diag"
    fail=1
  fi
}

TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

check "ignore, whitespace fields" 'x 1.0k\n'        0 'x 1000\n'        --invalid=ignore --to=si --field=-
check "warn, two bad fields"      'x 1.0k y 2.0k\n' 2 'x 1000 y 2000\n' --invalid=warn   --to=si --field=-
check "fail, two bad fields"      'x 1.0k y 2.0k\n' 2 'x 1000 y 2000\n' --invalid=fail   --to=si --field=-
check "ignore, -d: delimiter"     '1.0k:x:2.0k\n'   0 '1000:x:2000\n'   --invalid=ignore --to=si --field=- -d:
check "-z, newline in bad field"  'e '              1 'e\n'             -z --invalid=warn

[ $fail = 0 ] && { echo "all cases match GNU 9.12"; exit 0; }
exit 1
