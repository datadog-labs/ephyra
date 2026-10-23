#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]
# Builds binaryen's wasm-opt (Release + assertions, static) and the Rust driver
# (bynfuzz/bynexec, embedding wasmtime 50) into <build-dir>.
set -euo pipefail
SRC=$(realpath "$1")
mkdir -p "$2"
OUT=$(realpath "$2")
MODE=${3:-normal}
HERE=$(dirname "$(realpath "$0")")
WORK=/agent-run/work
export CCACHE_DIR=${CCACHE_DIR:-$WORK/.ccache}
export CARGO_HOME=${CARGO_HOME:-$WORK/.cargo}
JOBS=${JOBS:-14}

CFLAGS=""
if [ "$MODE" = coverage ]; then
  # Instrument binaryen's own sources only: third_party (LLVM DWARF support)
  # would triple the size of every profile.
  CFLAGS="-fprofile-instr-generate -fcoverage-mapping -fprofile-list=$HERE/profile-list.txt"
fi

mkdir -p "$OUT/binaryen" "$OUT/bin"
cmake -S "$SRC" -B "$OUT/binaryen" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DBYN_ENABLE_ASSERTIONS=ON -DBUILD_TESTS=OFF \
  -DBUILD_SHARED_LIBS=OFF -DBUILD_LLVM_DWARF=ON \
  -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ \
  -DCMAKE_C_COMPILER_LAUNCHER=ccache -DCMAKE_CXX_COMPILER_LAUNCHER=ccache \
  -DCMAKE_C_FLAGS="$CFLAGS" -DCMAKE_CXX_FLAGS="$CFLAGS" \
  -DCMAKE_EXE_LINKER_FLAGS="$CFLAGS" > "$OUT/cmake.log" 2>&1 || { tail -30 "$OUT/cmake.log"; exit 1; }
ninja -C "$OUT/binaryen" -j"$JOBS" wasm-opt wasm-dis > "$OUT/ninja.log" 2>&1 || { tail -40 "$OUT/ninja.log"; exit 1; }
for t in wasm-opt wasm-dis; do
  cp -f "$OUT/binaryen/bin/$t" "$OUT/bin/$t.tmp" && mv -f "$OUT/bin/$t.tmp" "$OUT/bin/$t"
done

# --initial-fuzz corpus from the target's lit tests (profiles of this step are discarded).
# Built aside and swapped in, so searches running on this build never see it empty.
rm -rf "$OUT/seeds.new" "$OUT/seeds.old"
LLVM_PROFILE_FILE=/dev/null JOBS=$JOBS bash "$HERE/make-seeds.sh" "$SRC" "$OUT/bin/wasm-opt" "$OUT/seeds.new" > "$OUT/seeds.log" 2>&1 || { tail "$OUT/seeds.log"; exit 1; }
[ -d "$OUT/seeds" ] && mv "$OUT/seeds" "$OUT/seeds.old"
mv "$OUT/seeds.new" "$OUT/seeds"
rm -rf "$OUT/seeds.old"

# The Rust driver is never instrumented: coverage is measured on wasm-opt only.
cargo build --release --manifest-path "$HERE/driver/Cargo.toml" \
  --target-dir "$OUT/cargo" > "$OUT/cargo.log" 2>&1 || { tail -40 "$OUT/cargo.log"; exit 1; }
for t in bynfuzz bynexec; do
  cp -f "$OUT/cargo/release/$t" "$OUT/bin/$t.tmp" && mv -f "$OUT/bin/$t.tmp" "$OUT/bin/$t"
done

if [ "$MODE" = coverage ]; then
  cat > "$OUT/coverage.json" <<JSON
{"objects": ["$OUT/bin/wasm-opt"],
 "profdata": "$(command -v llvm-profdata-18)",
 "cov": "$(command -v llvm-cov-18)"}
JSON
fi
echo "build ok: $OUT"
