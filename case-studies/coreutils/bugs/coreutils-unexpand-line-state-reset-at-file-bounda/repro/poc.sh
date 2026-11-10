#!/usr/bin/env bash
# PoC: unexpand restarts its line state (column, leading-blank flag) at every FILE operand.
# A line that does not end at the end of one file continues in the next, as with `cat f1 f2 | unexpand`.
# Usage: poc.sh <dir holding an `unexpand` executable>  (e.g. a symlink named unexpand to the uutils multicall binary)
# Exit 1 if the bug shows, 0 if output matches GNU coreutils 9.12, 2 on usage error.
B=${1:?usage: poc.sh <bin-dir>}
[ -x "$B/unexpand" ] || exit 2
export LC_ALL=C
T=$(mktemp -d) || exit 2; trap 'rm -rf "$T"' EXIT
cd "$T" || exit 2
bad=0
# 1. Eight leading spaces split 4 + 4 over two files. GNU: one tab, then x.
printf '    ' > a; printf '    x\n' > b
out=$("$B/unexpand" a b | sed -n l)
echo "unexpand a b     : $out   (GNU 9.12: \\tx\$)"
[ "$out" = '\tx$' ] || bad=1
# 2. With -a, "abcdef" then two spaces reach column 8. GNU: abcdef\tx.
printf 'abcdef' > c; printf '  x\n' > d
out=$("$B/unexpand" -a c d | sed -n l)
echo "unexpand -a c d  : $out   (GNU 9.12: abcdef\\tx\$)"
[ "$out" = 'abcdef\tx$' ] || bad=1
exit $bad
