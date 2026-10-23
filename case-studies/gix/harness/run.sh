#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>: one search process (restarted after a
# timeout finding or an abort, until the time is up).
set -u
BUILD=$(realpath "$1")
SEED=$2
SECS=$3
mkdir -p "$4"
OUT=$(realpath "$4")
mkdir -p "$OUT/findings"
export TMPDIR="$OUT/tmp"
mkdir -p "$TMPDIR"
end=$(( $(date +%s) + SECS ))
k=0
while :; do
  rem=$(( end - $(date +%s) ))
  [ "$rem" -gt 0 ] || break
  timeout -k 5 $(( rem + 120 )) "$BUILD/bin/gixfuzz" search $(( SEED + k * 7919 )) "$rem" "$OUT" 2>>"$OUT/search.log"
  rc=$?
  case $rc in
    0) break ;;
    3) ;;                                   # hang finding written; restart
    124|137) break ;;                       # our own outer timeout
    *) echo "search exited with $rc" >>"$OUT/search.log"
       "$BUILD/bin/gixfuzz" record-abort "$OUT" ;;
  esac
  k=$(( k + 1 ))
done
[ -f "$OUT/stats.json" ] || echo '{"cases": 0, "failures": 0}' > "$OUT/stats.json"
rm -rf "$TMPDIR"
