#!/usr/bin/env bash
# Minimal repro: printf's "invalid number" diagnostic ignores `2>/dev/null` (and other stderr
# redirections of the builtin), and printf exits 0 instead of 1.
# Usage: repro.sh /path/to/brush
# Exit 1 = bug reproduces, 0 = fixed (behaves like bash 5.3), 2 = cannot decide.
BRUSH=${1:?usage: repro.sh /path/to/brush}
[ -x "$BRUSH" ] || { echo "not executable: $BRUSH" >&2; exit 2; }
tmp=$(mktemp -d) || exit 2
trap 'rm -rf "$tmp"' EXIT

# 1. Diagnostic must go to the redirected stderr (here /dev/null), not to brush's own fd 2.
"$BRUSH" --norc --noprofile -c "printf '%d\n' B 2>/dev/null; echo rc=\$?" >"$tmp/out" 2>"$tmp/err"
out=$(cat "$tmp/out"); err=$(cat "$tmp/err")
echo "case 1: stdout=$(printf %q "$out") stderr=$(printf %q "$err")"
# Expected (bash 5.3): stdout "0\nrc=1", stderr empty.

# 2. Same inside a function whose stderr is redirected.
"$BRUSH" --norc --noprofile -c "f() { printf '%x\n' 12abc; }; f 2>/dev/null" >"$tmp/out2" 2>"$tmp/err2"
err2=$(cat "$tmp/err2")
echo "case 2: stderr=$(printf %q "$err2")"
# Expected: stderr empty.

bug=0
[ -n "$err" ]  && { echo "BUG: case 1 diagnostic bypassed 2>/dev/null"; bug=1; }
[ -n "$err2" ] && { echo "BUG: case 2 diagnostic bypassed f 2>/dev/null"; bug=1; }
[ "$out" != $'0\nrc=1' ] && { echo "BUG: case 1 expected stdout '0' then 'rc=1'"; bug=1; }
[ "$bug" -eq 0 ] && echo "OK: matches bash 5.3"
exit "$bug"
