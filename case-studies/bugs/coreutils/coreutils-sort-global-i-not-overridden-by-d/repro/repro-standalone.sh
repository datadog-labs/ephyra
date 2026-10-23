#!/usr/bin/env bash
# sort -d -i: -i must have no effect when -d is also given (GNU coreutils manual, sort -i).
#   repro-standalone.sh <path-to-uutils-sort-or-coreutils-multicall>
# Exit 1 = bug reproduces, 0 = does not, 2 = usage error.
# A multicall `coreutils` binary is called as `coreutils sort ...`.
set -u
BIN=${1:?usage: $0 <uutils sort or coreutils binary>}
[ -x "$BIN" ] || { echo "not executable: $BIN" >&2; exit 2; }
case $(basename "$BIN") in
  coreutils*) SORT=("$BIN" sort) ;;
  *)          SORT=("$BIN") ;;
esac

expected=$(printf '\tK\n7\n' | od -An -c)   # -d keeps the tab (a blank), so "\tK" < "7"
status=0
for args in "-d -i" "-i -d" "-di"; do
  # shellcheck disable=SC2086
  actual=$(printf '\tK\n7\n' | LC_ALL=C "${SORT[@]}" $args | od -An -c)
  if [ "$actual" = "$expected" ]; then
    echo "ok    sort $args"
  else
    echo "FAIL  sort $args"
    echo "  expected:$expected"
    echo "  actual:  $actual"
    status=1
  fi
done
exit $status
