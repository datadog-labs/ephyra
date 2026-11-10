#!/usr/bin/env bash
# usage: ./repro.sh [path/to/bun]   (default: bun on PATH)
# Bundles using-loop.js (default target=browser, which lowers `using`),
# runs the bundle, and compares with running the file directly.
# Exit 1 = bug reproduces, 0 = does not.
set -u
BUN=${1:-bun}
cd "$(dirname "$0")"
OUT=$(mktemp -d)
"$BUN" build using-loop.js --outfile="$OUT/out.js" >/dev/null || exit 2
direct=$("$BUN" using-loop.js)
bundled=$("$BUN" "$OUT/out.js")
rm -rf "$OUT"
echo "direct:";  echo "$direct"
echo "bundled:"; echo "$bundled"
[ "$direct" = "$bundled" ] && exit 0 || exit 1
