#!/bin/sh
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# cov.sh <seed> <secs> <out>: a search (TYZ_MAX_CASES caps it by programs) against the coverage-instrumented ty, then a
# per-file line-coverage report of ty's semantic crates (<out>/cov-files.txt).
set -e
. "$(dirname "$0")/env.sh"
RUFF=$HOME/Projects/ephyra-mono/ruff
BIN=$RUFF/target-cov/aarch64-apple-darwin/release/ty
TOOLS=$(cd "$RUFF" && rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin
seed=$1 secs=$2 out=$3
rm -rf "$out"; mkdir -p "$out/prof"
LLVM_PROFILE_FILE="$out/prof/%p-%m.profraw" TYZ_TY=$BIN TYZ_WORKERS=${TYZ_WORKERS:-6} TYZ_BATCH=${TYZ_BATCH:-40} \
  "$(dirname "$0")/target/release/tyz" search "$seed" "$secs" "$out/run" 2>/dev/null
"$TOOLS/llvm-profdata" merge -sparse "$out"/prof/*.profraw -o "$out/ty.profdata"
rm -rf "$out/prof"
"$TOOLS/llvm-cov" report "$BIN" -instr-profile="$out/ty.profdata" \
  --ignore-filename-regex='(\.cargo|rustc|/vendor/)' > "$out/cov-report.txt"
grep -E "^(ty_python_semantic|ty_python_core|ty_module_resolver)/|^TOTAL" "$out/cov-report.txt" \
  | awk '{print $1, $8, $9, $10}' > "$out/cov-files.txt"
tail -1 "$out/cov-report.txt"
