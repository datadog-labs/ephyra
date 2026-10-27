#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>: one search process (BBZ_WORKERS workers, default 2).
# Odd seeds run the AddressSanitizer build when build.sh made one.
set -euo pipefail
BUILD="$(cd "$1" && pwd)"
mkdir -p "$4"
BIN="$BUILD/bin/bbz"
if [ $(( $2 % 2 )) -eq 1 ] && [ -x "$BUILD/bin/bbz-asan" ]; then
  BIN="$BUILD/bin/bbz-asan"
  export BBZ_HANG_S="${BBZ_HANG_S:-90}"
  # leaks are bbtree-store's pages-out check's job; keep ASan to memory errors
  export ASAN_OPTIONS="${ASAN_OPTIONS:-detect_leaks=0:abort_on_error=1}"
fi
# The coverage build runs 8-64x slower than the plain one (measured per property family,
# window 3 of the first agent run): bulk cases of 100k entries take 26-40 s there and well
# under 1 s plain. A 30 s deadline made false crash__hang findings; repros run on the plain
# build with its own 30 s deadline, so a real hang still shows there.
if [ -f "$BUILD/coverage.json" ]; then
  export BBZ_HANG_S="${BBZ_HANG_S:-600}"
fi
exec "$BIN" search "$2" "$3" "$(cd "$4" && pwd)"
