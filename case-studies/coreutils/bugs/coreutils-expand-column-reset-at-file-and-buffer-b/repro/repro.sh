#!/usr/bin/env bash
# expand: the column is reset at every input file (and is not advanced over a
# tab-free 8 KiB read buffer), so later tabs land on the wrong stop.
#
# usage: repro.sh <path-to-uutils-expand | path-to-uutils-coreutils-multicall>
# exit 1 = bug reproduces, 0 = all cases match GNU coreutils 9.12, 2 = usage error
# Expected outputs below were produced by GNU coreutils 9.12 expand.
set -u
BIN=${1:?usage: repro.sh <expand binary or coreutils multicall binary>}
[ -x "$BIN" ] || { echo "not executable: $BIN" >&2; exit 2; }
BIN=$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
case $(basename "$BIN") in coreutils*) EXPAND=("$BIN" expand) ;; *) EXPAND=("$BIN") ;; esac

T=$(mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT
cd "$T" || exit 2
export LC_ALL=C
fail=0

check() { # name, expected-file, actual-file
  if cmp -s "$2" "$3"; then echo "ok:   $1"
  else
    echo "FAIL: $1 (last 16 bytes, od -c)"
    echo "  expected:"; tail -c 16 "$2" | od -An -c
    echo "  actual:";   tail -c 16 "$3" | od -An -c
    fail=1
  fi
}

# 1. File 1 does not end in a newline; the tab at the start of file 2 is at column 1.
printf 'a' > f1
printf '\tb\n' > f2
"${EXPAND[@]}" f1 f2 > out1
printf 'a       b\n' > exp1            # 'a' + 7 spaces + 'b'
check "expand f1 f2" exp1 out1

# 2. Same with -i: the tab in file 2 is not initial (after 'a'), so it stays a tab.
"${EXPAND[@]}" -i f1 f2 > out2
printf 'a\tb\n' > exp2
check "expand -i f1 f2" exp2 out2

# 3. One file: 8190 'a' then 'bcd<TAB>x'. The first 8 KiB read holds no tab or
#    newline. The tab is at column 8193, so with -t3 it becomes 3 spaces.
{ head -c 8190 /dev/zero | tr '\0' a; printf 'bcd\tx\n'; } > big
{ head -c 8190 /dev/zero | tr '\0' a; printf 'bcd   x\n'; } > exp3
"${EXPAND[@]}" -t3 < big > out3
check "expand -t3 < big (8190 'a' + 'bcd<TAB>x')" exp3 out3

exit $fail
