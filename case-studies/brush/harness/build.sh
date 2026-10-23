#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]
# With "coverage", brush is built with -C instrument-coverage (own target dir) and
# <build-dir>/coverage.json names the instrumented binary and the matching llvm tools.
# Builds brush (optimised, with debug assertions, overflow checks and panic=unwind so
# panics are visible), the fuzzer (fuzzer/), and copies the bash 5.3 reference into
# <build-dir>. Incremental: cargo target dirs live under <build-dir>.
set -euo pipefail
SRC=$(realpath "$1")
mkdir -p "$2"
OUT=$(realpath "$2")
MODE=${3:-}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REF_BASH=${REF_BASH:-/agent-run/work/ref/bash-5.3/bin/bash}
export RUSTUP_TOOLCHAIN=stable   # rust-toolchain.toml pins 1.99.1 == installed stable; avoid rustup sync
export CARGO_HOME=${CARGO_HOME:-/agent-run/work/.cargo}

# One target dir per source path: cargo treats a different tree with older mtimes as fresh
# and would keep a binary (and coverage map) compiled from another tree.
SRCKEY=$(printf '%s' "$SRC" | sha1sum | cut -c1-10)
BRUSH_TD="$OUT/target-brush-$SRCKEY"
RF=${RUSTFLAGS:-}
if [ "$MODE" = coverage ]; then
  BRUSH_TD="$OUT/target-brush-cov-$SRCKEY"
  RF="$RF -C instrument-coverage"
fi
(
  cd "$SRC"
  RUSTFLAGS="$RF" LLVM_PROFILE_FILE="$OUT/build-profraw/%m_%p.profraw" \
  CARGO_PROFILE_RELEASE_LTO=false \
  CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
  CARGO_PROFILE_RELEASE_PANIC=unwind \
  CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true \
  CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true \
  CARGO_PROFILE_RELEASE_STRIP=false \
    cargo build --release --locked --target-dir "$BRUSH_TD" -p brush-shell
)
cp -f "$BRUSH_TD/release/brush" "$OUT/brush.new" && mv -f "$OUT/brush.new" "$OUT/brush"

cargo build --release --manifest-path "$HERE/fuzzer/Cargo.toml" --target-dir "$OUT/target-fuzzer"
cp -f "$OUT/target-fuzzer/release/brushfuzz" "$OUT/brushfuzz.new" && mv -f "$OUT/brushfuzz.new" "$OUT/brushfuzz"

cp -f "$REF_BASH" "$OUT/bash.new" && mv -f "$OUT/bash.new" "$OUT/bash"
"$OUT/bash" --version | head -1
"$OUT/brush" --version | head -1

if [ "$MODE" = coverage ]; then
  SYSROOT=$(rustc --print sysroot)
  HOST=$(rustc -vV | sed -n 's/^host: //p')
  TOOLS="$SYSROOT/lib/rustlib/$HOST/bin"
  [ -x "$TOOLS/llvm-profdata" ] && [ -x "$TOOLS/llvm-cov" ] || { echo "llvm-tools missing in $TOOLS" >&2; exit 1; }
  printf '{"objects": ["%s"], "profdata": "%s", "cov": "%s"}\n' \
    "$OUT/brush" "$TOOLS/llvm-profdata" "$TOOLS/llvm-cov" > "$OUT/coverage.json"
  cat "$OUT/coverage.json"
fi
