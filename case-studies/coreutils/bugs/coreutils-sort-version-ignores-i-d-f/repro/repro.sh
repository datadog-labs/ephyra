#!/usr/bin/env bash
# sort -V ignores -i / -d / -f.
# Usage: repro.sh <path-to-sort>   e.g. repro.sh ./target/debug/sort  (or "coreutils sort" via a symlink named sort)
# Exit 1 if any case differs from the expected (GNU 9.12) result, 0 if all match.
SORT=${1:?usage: repro.sh <path-to-uutils-sort>}
export LC_ALL=C
fail=0
check() { # $1 = description, $2 = expected, $3 = actual
  if [ "$2" = "$3" ]; then echo "ok    $1"; else
    echo "FAIL  $1"; echo "      expected: $(printf %q "$2")"; echo "      actual:   $(printf %q "$3")"; fail=1; fi
}

# 1. -i: the tab is nonprinting, so keys are "0" and "" -> not sorted, exit 1
printf '0\n\t\n' | "$SORT" -c -iV 2>/dev/null; check "printf '0\\n\\t\\n' | sort -c -iV  (exit status)" 1 $?

# 2. -f: fold case, so "a" < "B"
check "printf 'B\\na\\n' | sort -fV" $'a\nB' "$(printf 'B\na\n' | "$SORT" -fV)"

# 3. -d: '-' is not alphanumeric, so keys are "2" and "1" -> 1 sorts first
check "printf -- '-2\\n1\\n' | sort -dV" $'1\n-2' "$(printf -- '-2\n1\n' | "$SORT" -dV)"

# 4. per-key: -k1fV behaves like -fV
check "printf 'B\\na\\n' | sort -k1fV" $'a\nB' "$(printf 'B\na\n' | "$SORT" -k1fV)"

exit $fail
