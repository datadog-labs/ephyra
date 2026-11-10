#!/usr/bin/env bash
# od -S with -Ax: string offsets should be 6 hex digits, as in od's normal -Ax dump.
#   repro.sh [path-to-od]   (default: od on PATH; for the multicall binary, pass a
#                            symlink named od, or set OD="coreutils od")
# exit 1 = bug reproduces, 0 = fixed, 2 = cannot decide
OD=${OD:-${1:-od}}
run() { printf "$1" | LC_ALL=C $OD "${@:2}"; }

got_s=$(run 'a\0' -Ax -S1) || exit 2
got_dump=$(run 'a' -Ax -c | head -1) || exit 2
echo "od -Ax -S1 : '$got_s'   (expected '000000 a')"
echo "od -Ax -c  : '$got_dump'   (normal dump, same address radix)"
[ "$got_s" = "000000 a" ] && exit 0
[ "$got_s" = "0000000 a" ] && exit 1
exit 2
