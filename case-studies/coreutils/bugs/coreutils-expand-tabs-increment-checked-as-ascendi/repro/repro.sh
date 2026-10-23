#!/usr/bin/env bash
# expand rejects a trailing '/N' or '+N' increment that is not larger than the previous tab stop.
#
#   repro.sh <path-to-uutils-coreutils-binary-or-expand> [<gnu-expand>]
#
# e.g. repro.sh target/debug/coreutils
#      repro.sh target/debug/coreutils /usr/bin/expand
# Spaces in stdout are shown as '.'.
# Exit 1 = bug reproduces, 0 = it does not, 2 = usage error.
set -u
BIN=${1:?usage: repro.sh <uutils coreutils or expand binary> [gnu expand]}
GNU=${2:-}
[ -x "$BIN" ] || { echo "not executable: $BIN" >&2; exit 2; }
case $(basename "$BIN") in
  expand) EXPAND=("$BIN") ;;
  *)      EXPAND=("$BIN" expand) ;;
esac
export LC_ALL=C
T=$(mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT

bad=0
check() { # $1 = --tabs value, $2 = input, $3 = expected stdout
  echo "== printf '$2' | expand --tabs=$1"
  printf "$2" | "${EXPAND[@]}" --tabs="$1" > "$T/out" 2> "$T/err"; rc=$?
  printf 'stdout: [%s]\n' "$(tr ' ' . < "$T/out")"
  printf 'stderr: '; cat "$T/err"; echo "exit: $rc"
  if [ -n "$GNU" ]; then
    printf "$2" | "$GNU" --tabs="$1" > "$T/gout" 2>&1; grc=$?
    printf 'gnu:    [%s] (exit %s)\n' "$(tr ' ' . < "$T/gout")" "$grc"
  fi
  if [ "$rc" = 0 ] && [ ! -s "$T/err" ] && [ "$(cat "$T/out")" = "$3" ]; then
    echo "-> ok"
  else
    echo "-> BUG (expected stdout [$(printf %s "$3" | tr ' ' .)], no stderr, exit 0)"; bad=1
  fi
}

# Tab stop at 2, then every multiple of 2: stops 2,4,6,...
check '2,/2' 'a\tb\tc\td' 'a b c d'
# Tab stop at 4, then every 2 columns after it: stops 4,6,8,...
check '4,+2' '\ta\tb\tc' '    a b c'
# Control (stops 2,3,6,9,...): increment larger than the last stop already works.
check '2,/3' 'a\tb\tc\td' 'a b   c  d'

[ $bad = 1 ] && exit 1
exit 0
