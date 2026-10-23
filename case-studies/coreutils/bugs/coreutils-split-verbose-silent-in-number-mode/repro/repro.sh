#!/usr/bin/env bash
# Reproduce one uudiff finding (uutils vs GNU coreutils 9.12).
#   repro.sh <build-dir>            exit 1 = reproduces, 0 = does not, 2 = cannot decide
#   repro.sh --record <gnu-bin-dir> (re)write expected/ from the reference (the harness did this once)
# Everything the case needs is here: case/args (NUL-separated argv after the util name),
# case/stdin, case/files/ (the starting working directory), expected/ (GNU's behaviour).
UTIL=split
ASPECT=stdout   # panic | signal | timeout | exit | stdout | files | stderr
H=$(cd "$(dirname "$0")" && pwd)

run() { # $1 = bin dir holding $UTIL, $2 = result dir
  local T rc
  T=$(mktemp -d) || exit 2
  mkdir "$T/w" && cp -a "$H/case/files/." "$T/w/" || exit 2
  local -a ARGS=()
  mapfile -d '' ARGS < "$H/case/args"
  ( cd "$T/w" && ulimit -f 16384 && exec /usr/bin/timeout -s KILL 10 \
      /usr/bin/env -i PATH="$1" LC_ALL=C TZ=UTC0 "$UTIL" "${ARGS[@]}" ) \
    < "$H/case/stdin" > "$2/stdout" 2> "$2/stderr"
  rc=$?
  echo "$rc" > "$2/exit"
  if [ -s "$2/stderr" ]; then echo 1; else echo 0; fi > "$2/stderr_nonempty"
  ( cd "$T/w" && find . -mindepth 1 -printf '%P %y %m %l\n' | LC_ALL=C sort &&
    find . -type f -print0 | LC_ALL=C sort -z | xargs -0 -r md5sum ) > "$2/tree"
  chmod -R u+rwx "$T" 2>/dev/null; rm -rf "$T"
}

if [ "${1:-}" = --record ]; then
  mkdir -p "$H/expected" && run "$(cd "$2" && pwd)" "$H/expected"; exit 0
fi
B=${1:?usage: repro.sh <build-dir>}
[ -x "$B/bin/$UTIL" ] && [ -f "$H/expected/exit" ] || exit 2
R=$(mktemp -d) || exit 2
run "$(cd "$B/bin" && pwd)" "$R"
rc=$(cat "$R/exit"); want=$(cat "$H/expected/exit")
case $ASPECT in
  panic)   grep -q 'panicked at' "$R/stderr" || [ "$rc" = 101 ] ;;
  signal)  [ "$rc" -gt 128 ] && [ "$rc" != 137 ] && [ "$rc" != "$want" ] ;;
  timeout) [ "$rc" = 137 ] ;;
  exit)    [ "$rc" != "$want" ] ;;
  stdout)  ! cmp -s "$R/stdout" "$H/expected/stdout" ;;
  files)   ! cmp -s "$R/tree" "$H/expected/tree" ;;
  stderr)  ! cmp -s "$R/stderr_nonempty" "$H/expected/stderr_nonempty" ;;
  *) rm -rf "$R"; exit 2 ;;
esac
found=$?
rm -rf "$R"
[ $found = 0 ] && exit 1
exit 0
