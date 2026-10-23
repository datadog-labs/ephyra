#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]: builds duckdb (relassert: RelWithDebInfo + FORCE_ASSERT, so D_ASSERT
# raises INTERNAL errors) and the Rust harness binary into <build-dir>. Incremental and re-runnable.
set -euo pipefail
SRC=$(realpath "$1"); mkdir -p "$2"; B=$(realpath "$2")
HERE=$(cd "$(dirname "$0")" && pwd)
export CCACHE_DIR=${CCACHE_DIR:-/agent-run/work/ccache}
# share the cache across source trees and build dirs (mutant and patched builds)
export CCACHE_BASEDIR=${CCACHE_BASEDIR:-/agent-run} CCACHE_NOHASHDIR=1
JOBS=${JOBS:-14}
MODE=${3:-}
# default: gcc. coverage: clang 18 with LLVM source-based coverage (libduckdb.so is the instrumented
# object; the harness binary is not instrumented). The worker child exits cleanly at the end of a run so
# the profile is written to $LLVM_PROFILE_FILE.
if [ "$MODE" = coverage ]; then
  # no path rewriting: a cache hit must never bring in objects compiled from another tree
  unset CCACHE_BASEDIR CCACHE_NOHASHDIR
  LLVM=/usr/lib/llvm-18/bin
  CF="-fprofile-instr-generate -fcoverage-mapping"
  COMPILERS=(-DCMAKE_C_COMPILER=$LLVM/clang -DCMAKE_CXX_COMPILER=$LLVM/clang++
    "-DCMAKE_C_FLAGS=$CF" "-DCMAKE_CXX_FLAGS=$CF"
    "-DCMAKE_EXE_LINKER_FLAGS=-fprofile-instr-generate" "-DCMAKE_SHARED_LINKER_FLAGS=-fprofile-instr-generate")
else
  COMPILERS=(-DCMAKE_C_COMPILER=gcc -DCMAKE_CXX_COMPILER=g++)
fi
# a build dir configured for the other mode is reconfigured from scratch
if [ -f "$B/cmake/CMakeCache.txt" ]; then
  if grep -q "CMAKE_CXX_COMPILER:.*clang" "$B/cmake/CMakeCache.txt"; then was=coverage; else was=; fi
  [ "$was" = "$MODE" ] || rm -rf "$B/cmake"
fi
# a build dir configured from another source tree is reconfigured too (coverage maps carry the
# absolute source paths of the tree that was compiled)
if [ -f "$B/cmake/CMakeCache.txt" ] && ! grep -qx "CMAKE_HOME_DIRECTORY:INTERNAL=$SRC" "$B/cmake/CMakeCache.txt"; then
  rm -rf "$B/cmake"
fi
if [ ! -f "$B/cmake/build.ninja" ]; then
  cmake -G Ninja -S "$SRC" -B "$B/cmake" -DCMAKE_BUILD_TYPE=RelWithDebInfo -DFORCE_ASSERT=1 \
    -DCMAKE_C_COMPILER_LAUNCHER=ccache -DCMAKE_CXX_COMPILER_LAUNCHER=ccache \
    "${COMPILERS[@]}" \
    -DBUILD_EXTENSIONS="core_functions;json;icu" -DBUILD_UNITTESTS=0 -DENABLE_SANITIZER=0 -DENABLE_UBSAN=0
fi
ninja -C "$B/cmake" -j"$JOBS" src/libduckdb.so duckdb
[ "${SKIP_HARNESS:-0}" = 1 ] && exit 0
export DUCKDB_LIB_DIR="$B/cmake/src" DUCKDB_INCLUDE_DIR="$SRC/src/include"
CARGO_TARGET_DIR="$B/cargo" cargo build --release --manifest-path "$HERE/Cargo.toml"
cp "$B/cargo/release/duckharness" "$B/duckharness"
if [ "$MODE" = coverage ]; then
  cat > "$B/coverage.json" <<JSON
{"objects": ["$B/cmake/src/libduckdb.so"], "profdata": "$LLVM/llvm-profdata", "cov": "$LLVM/llvm-cov"}
JSON
fi
echo "built $B/duckharness against $B/cmake/src/libduckdb.so"
