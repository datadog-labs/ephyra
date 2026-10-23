#!/usr/bin/env bash
# Standalone repro: break/continue in brush ignore how many loops enclose them.
# Usage: ./repro.sh /path/to/brush      (default: brush on PATH)
# Exit 1 if brush's results differ from the bash 5.x results listed below, 0 if they match.
BRUSH=${1:-brush}
fail=0
check() { # name, script, expected
  local actual
  actual=$("$BRUSH" --norc --noprofile -c "$2" 2>/dev/null)
  if [ "$actual" == "$3" ]; then echo "ok   $1"; else
    echo "FAIL $1"; echo "  script:   $2"; echo "  expected: $(printf %q "$3")"; echo "  actual:   $(printf %q "$actual")"; fail=1; fi
}
# 1. break in a function called outside any loop (bash: warning on stderr, status 0, function goes on)
check func-break 'f() { break; echo "f went on: $?"; }; f; echo "rc=$?"' $'f went on: 0\nrc=0'
# 2. break/continue at top level (bash: warning, status 0, rest of the line runs)
check toplevel 'break; echo "after break: $?"
continue; echo "after continue: $?"' $'after break: 0\nafter continue: 0'
# 3. a function's break must not leave the caller's loop
check caller-loop 'f() { break; echo "f went on: $?"; }; for i in 1 2; do f; echo "i=$i"; done' $'f went on: 0\ni=1\nf went on: 0\ni=2'
# 4. break N with N larger than the loop depth stops at the outermost loop
check break-N 'for i in 1 2; do for j in a b; do break 5; done; echo "not reached"; done
echo "after break: $?"' 'after break: 0'
exit $fail
