#!/usr/bin/env bash
# join: trailing blanks add an empty field; -i prints the first line's key for a whole group.
# Usage: repro.sh [JOIN]   JOIN = path to a join binary (default: "join" on PATH).
#   e.g. repro.sh target/debug/join   or   repro.sh "target/debug/coreutils join"
# Expected bytes are GNU coreutils 9.12's output. Exit 1 if any case differs (bug present), 0 if none.
JOIN=${1:-join}
T=$(mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT
cd "$T" || exit 2
export LC_ALL=C
printf '1 \n'     > t1;  printf '1\n'   > t2     # trailing blank on file1's line
printf '1 2 \t\n' > u1;  printf '1\n'   > u2
printf '1\n'      > z1;  printf '1'     > z2     # with -z the '\n' is a blank
printf 'B\nb\n'   > i1;  printf 'B\n'   > i2
printf 'B x\nb y\n' > k1; printf 'b z\n' > k2
fail=0
check() { # $1 = expected (printf format), rest = join args
  local want got; want=$(printf "$1" | od -An -c); shift
  got=$($JOIN "$@" | od -An -c)
  if [ "$got" = "$want" ]; then echo "ok    join $*"
  else echo "FAIL  join $*"; echo "   expected:$want"; echo "   actual:  $got"; fail=1; fi
}
check '1\n'            t1 t2
check '1 2\n'          u1 u2
check '1 2\n'          -o auto -e X u1 u2
check '1\0'            -z z1 z2
check 'B\nb\n'         -i i1 i2
check 'B x z\nb y z\n' -i k1 k2
check 'B\nb\n'         -i -o 0 k1 k2
exit $fail
