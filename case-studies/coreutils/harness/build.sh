#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]: build uutils coreutils (dev profile, opt-level 1:
# debug assertions + overflow checks + panic=unwind) and the ephyra search driver.
set -euo pipefail
SRC=$(realpath "$1"); mkdir -p "$2"; OUT=$(realpath "$2")
HERE=$(cd "$(dirname "$0")" && pwd)
export CARGO_HOME=${CARGO_HOME:-/agent-run/work/.cargo}

MODE=${3:-}
if [ "$MODE" = coverage ]; then
  # LLVM source-based coverage; build scripts run instrumented too, keep their
  # profiles out of the source tree.
  mkdir -p "$OUT/build-profraw"
  RUSTFLAGS="${RUSTFLAGS:-} -C instrument-coverage" LLVM_PROFILE_FILE="$OUT/build-profraw/%p-%m.profraw" \
  CARGO_PROFILE_DEV_OPT_LEVEL=1 CARGO_PROFILE_DEV_DEBUG=line-tables-only \
  CARGO_TARGET_DIR="$OUT" cargo build --quiet --manifest-path "$SRC/Cargo.toml" --features unix -j10
else
  CARGO_PROFILE_DEV_OPT_LEVEL=1 CARGO_PROFILE_DEV_DEBUG=line-tables-only \
  CARGO_TARGET_DIR="$OUT" cargo build --quiet --manifest-path "$SRC/Cargo.toml" --features unix -j10
fi

# argv0 dispatch: bin/<util> -> coreutils, so messages say '<util>:' as GNU's do.
mkdir -p "$OUT/bin"
ln -sfn ../debug/coreutils "$OUT/bin/coreutils"
for u in $("$OUT/debug/coreutils" --list); do
  [ "$u" = "[" ] && continue
  ln -sfn ../debug/coreutils "$OUT/bin/$u"
done

# The search driver (does not depend on the target's source).
CARGO_TARGET_DIR="$OUT/driver" cargo build --quiet --release --manifest-path "$HERE/driver/Cargo.toml"
cp "$OUT/driver/release/uudiff" "$OUT/uudiff.new" && mv "$OUT/uudiff.new" "$OUT/uudiff"
if [ "$MODE" = coverage ]; then
  SYSROOT=$(rustc --print sysroot)
  TOOLS=$(dirname "$(ls "$SYSROOT"/lib/rustlib/*/bin/llvm-profdata | head -1)")
  [ -x "$TOOLS/llvm-profdata" ] && [ -x "$TOOLS/llvm-cov" ] || { echo "llvm-tools missing" >&2; exit 1; }
  printf '{"objects": ["%s"], "profdata": "%s", "cov": "%s"}\n' \
    "$OUT/debug/coreutils" "$TOOLS/llvm-profdata" "$TOOLS/llvm-cov" > "$OUT/coverage.json"
fi
echo "build ok: $OUT"
