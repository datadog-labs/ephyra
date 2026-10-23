#!/usr/bin/env bash
# brush deadlocks when a command substitution's builtin writes more than a pipe
# buffer (~64 KiB) after running a nested command substitution.
# Usage: repro.sh [path-to-brush]   (default: brush on PATH)
# Exit 1 = brush hung (no result in 20s), 0 = brush printed "ok", 2 = other.
BRUSH=${1:-brush}
script='for i in {1..200}; do x=$(printf "%070000d" $(echo 0)); done; echo ok'
out=$(timeout 20 "$BRUSH" --norc --noprofile -c "$script" </dev/null 2>&1)
rc=$?
if [ "$rc" = 124 ]; then echo "HANG: brush did not finish within 20s"; exit 1; fi
if [ "$rc" = 0 ] && [ "$out" = ok ]; then echo "OK: brush printed 'ok'"; exit 0; fi
echo "UNEXPECTED: rc=$rc output=$out"; exit 2
