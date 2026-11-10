#!/usr/bin/env bash
# comm compares each line together with its trailing newline.
#
#   repro.sh <path-to-uutils-coreutils-binary-or-comm> [<gnu-comm>]
#
# e.g. repro.sh target/debug/coreutils
#      repro.sh target/debug/coreutils /usr/bin/comm
# Exit 1 = bug reproduces, 0 = it does not, 2 = usage error.
set -u
BIN=${1:?usage: repro.sh <uutils coreutils or comm binary> [gnu comm]}
GNU=${2:-}
[ -x "$BIN" ] || { echo "not executable: $BIN" >&2; exit 2; }
BIN=$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
[ -z "$GNU" ] || GNU=$(cd "$(dirname "$GNU")" && pwd)/$(basename "$GNU")
case $(basename "$BIN") in
  comm) COMM=("$BIN") ;;
  *)    COMM=("$BIN" comm) ;;
esac
T=$(mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT
cd "$T" || exit 2
export LC_ALL=C

bad=0

# Case 1: lines "" and "\t" in that order are sorted in the C locale
# (this is exactly what `printf '\t\n\n' | LC_ALL=C sort` prints).
: > empty
printf '\n\t\n' > sorted
echo "== case 1: comm empty sorted   (sorted = \"\" then \"\\t\")"
"${COMM[@]}" empty sorted > out1 2> err1; rc=$?
printf 'stdout: '; od -An -c out1 | tr -s ' '; echo
printf 'stderr: '; cat err1; echo "exit: $rc"
echo "expected: stdout \"\\t\\n\\t\\t\\n\" (both lines in column 2), no stderr, exit 0"
if [ "$rc" != 0 ] || [ -s err1 ]; then bad=1; echo "-> BUG"; else echo "-> ok"; fi

# Case 2: "a" < "a\tb", so the line from file 1 must be printed first.
printf 'a\n' > f1
printf 'a\tb\n' > f2
echo
echo "== case 2: comm f1 f2   (f1 = \"a\", f2 = \"a\\tb\")"
"${COMM[@]}" f1 f2 > out2 2> err2; rc=$?
printf 'stdout: '; od -An -c out2 | tr -s ' '; echo
printf 'stderr: '; cat err2; echo "exit: $rc"
printf 'a\n\ta\tb\n' > want2
echo "expected: stdout \"a\\n\\ta\\tb\\n\", no stderr, exit 0"
if ! cmp -s out2 want2 || [ "$rc" != 0 ] || [ -s err2 ]; then bad=1; echo "-> BUG"; else echo "-> ok"; fi

if [ -n "$GNU" ]; then
  echo
  echo "== reference: $GNU"
  "$GNU" empty sorted | od -An -c; echo "exit: ${PIPESTATUS[0]}"
  "$GNU" f1 f2 | od -An -c; echo "exit: ${PIPESTATUS[0]}"
fi
exit $bad
