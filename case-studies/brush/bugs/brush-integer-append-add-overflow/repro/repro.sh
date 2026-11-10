#!/usr/bin/env bash
# Minimal repro: `+=` on a `declare -i` variable whose sum overflows i64.
# Usage: repro.sh /path/to/brush   (a debug build, i.e. plain `cargo build`, or any build with overflow-checks)
# Exit 1 = bug reproduces (brush panics or prints a value other than bash's), 0 = fixed, 2 = cannot decide.
BRUSH=${1:?usage: repro.sh /path/to/brush}
[ -x "$BRUSH" ] || { echo "not executable: $BRUSH" >&2; exit 2; }
script='declare -i n=-1
n+=-9223372036854775808
echo "$n"'
expected='9223372036854775807'   # bash 5.3 output (64-bit wraparound)
out=$("$BRUSH" --norc --noprofile -c "$script" 2>"${TMPDIR:-/tmp}/repro.stderr.$$"); rc=$?
err=$(cat "${TMPDIR:-/tmp}/repro.stderr.$$"); rm -f "${TMPDIR:-/tmp}/repro.stderr.$$"
echo "exit=$rc stdout=$out"; [ -n "$err" ] && echo "stderr: $err"
if [ "$rc" -eq 0 ] && [ "$out" = "$expected" ]; then echo "OK: matches bash"; exit 0; fi
echo "BUG: expected '$expected' with exit 0"; exit 1
