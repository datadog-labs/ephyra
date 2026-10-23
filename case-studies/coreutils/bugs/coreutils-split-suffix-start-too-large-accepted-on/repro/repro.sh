#!/usr/bin/env bash
# split accepts a suffix start value that does not fit the fixed suffix length when input is empty.
# Usage: repro.sh <dir containing a `split` binary (or a uutils multicall binary named split)>
# Exit 1 = bug reproduces, 0 = does not.
set -u
BIN=${1:?usage: repro.sh <bin-dir-with-split>}
bug=0
check() { # $1 = stdin kind, rest = args
  local kind=$1; shift
  local d out rc
  d=$(mktemp -d)
  if [ "$kind" = empty ]; then
    out=$(cd "$d" && env -i PATH="$BIN" LC_ALL=C split "$@" </dev/null 2>&1); rc=$?
  else
    out=$(cd "$d" && printf 'a\n' | env -i PATH="$BIN" LC_ALL=C split "$@" 2>&1); rc=$?
  fi
  printf '%-32s stdin=%-5s exit=%s files=[%s] stderr=%s\n' "split $*" "$kind" "$rc" \
    "$(ls -A "$d" | tr '\n' ' ')" "$(printf '%s' "$out" | head -1)"
  rm -rf "$d"
  return $rc
}
for args in "--numeric-suffixes=100" "-a 1 --hex-suffixes=15"; do
  # shellcheck disable=SC2086
  check data $args
  # shellcheck disable=SC2086
  check empty $args && bug=1      # expected: exit 1 with the "too large" error, as for non-empty input
done
check empty -a 1 --hex-suffixes=f || echo "(control: 'f' fits in one hex digit and should succeed)"
[ $bug = 1 ] && { echo "REPRODUCES: empty input accepted an out-of-range suffix start"; exit 1; }
echo "does not reproduce"; exit 0
