#!/usr/bin/env bash
# Usage: ./run.sh [path-to-bun]   (default: bun on PATH)
# Prints the parsed object. Expected {"r":{"@v":"a\n"}}; the bug prints {"r":{"@v":"a"}}.
# Exit 1 if the bug reproduces, 0 if not.
cd "$(dirname "$0")"
BUN=${1:-bun}
out=$("$BUN" repro.js) || exit 2
echo "$out"
[ "$out" = '{"r":{"@v":"a\n"}}' ] && exit 0
[ "$out" = '{"r":{"@v":"a"}}' ] && exit 1
exit 2
