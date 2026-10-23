#!/usr/bin/env bash
# uniq -f1 must treat "a\rb" and "c\rd" as one field each ('\r' is not a blank),
# so after skipping one field both keys are empty and only the first line is printed.
#   poc.sh [uniq-binary]     (default: uniq on PATH)
# exit 1 = bug reproduces, 0 = correct behaviour
UNIQ=${1:-uniq}
out=$(printf 'a\rb\nc\rd\n' | LC_ALL=C "$UNIQ" -f1 | od -An -c | tr -s ' ')
echo "output:$out"
want=$(printf 'a\rb\n' | od -An -c | tr -s ' ')
[ "$out" = "$want" ] && { echo "OK: one line (expected)"; exit 0; }
echo "BUG: '\\r' was treated as a field separator"; exit 1
