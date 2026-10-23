#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]
# Builds the dfz harness binary against the DataFusion tree in <src-dir>.
# With "coverage": instrumented for LLVM source-based coverage (-C instrument-coverage) and
# <build-dir>/coverage.json names the binary and the toolchain's llvm-profdata / llvm-cov.
# Everything (crate copy, cargo target dir, final binary) goes into <build-dir>.
set -euo pipefail
SRC="$(cd "$1" && pwd)"
mkdir -p "$2"
BUILD="$(cd "$2" && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"
MODE="${3:-}"

export RUSTUP_TOOLCHAIN=stable          # rust-toolchain.toml pins 1.99.0 which rustup cannot install here
export CARGO_HOME="${CARGO_HOME:-/agent-run/work/.cargo}"
EPHYRA="${EPHYRA:-$(cd "$(dirname "$0")/../../.." && pwd)}"

CRATE="$BUILD/crate"
mkdir -p "$CRATE/src"
# copy sources only when changed so cargo's fingerprints stay valid
for f in "$HERE"/crate/src/*.rs; do
  cmp -s "$f" "$CRATE/src/$(basename "$f")" || cp "$f" "$CRATE/src/"
done
sed -e "s#@SRC@#$SRC#g" -e "s#@EPHYRA@#$EPHYRA#g" "$HERE/crate/Cargo.toml.in" > "$CRATE/Cargo.toml.new"
cmp -s "$CRATE/Cargo.toml.new" "$CRATE/Cargo.toml" || mv "$CRATE/Cargo.toml.new" "$CRATE/Cargo.toml"
rm -f "$CRATE/Cargo.toml.new"
# resolve the same dependency versions as the target's own workspace
[ -f "$CRATE/Cargo.lock" ] || cp "$SRC/Cargo.lock" "$CRATE/Cargo.lock"

if [ "$MODE" = coverage ]; then
  SYSROOT="$(rustc --print sysroot)"
  HOST="$(rustc -vV | sed -n 's/^host: //p')"
  TOOLS="$SYSROOT/lib/rustlib/$HOST/bin"
  [ -x "$TOOLS/llvm-profdata" ] && [ -x "$TOOLS/llvm-cov" ] || { echo "llvm-tools missing in $TOOLS" >&2; exit 1; }
  # an explicit --target keeps the flag off build scripts and proc macros
  export RUSTFLAGS="${RUSTFLAGS:-} -C instrument-coverage"
  # instrumented build scripts would otherwise drop .profraw files into the source tree
  export LLVM_PROFILE_FILE="$BUILD/profraw-build/%m_%p.profraw"
  cargo build --manifest-path "$CRATE/Cargo.toml" --profile fuzz --target "$HOST" --target-dir "$BUILD/target" 2>&1 | tail -n 30
  OUT="$BUILD/target/$HOST/fuzz/dfz"
else
  cargo build --manifest-path "$CRATE/Cargo.toml" --profile fuzz --target-dir "$BUILD/target" 2>&1 | tail -n 30
  OUT="$BUILD/target/fuzz/dfz"
fi
mkdir -p "$BUILD/bin"
cp "$OUT" "$BUILD/bin/dfz.new"
mv "$BUILD/bin/dfz.new" "$BUILD/bin/dfz"
if [ "$MODE" = coverage ]; then
  printf '{"objects": ["%s"], "profdata": "%s", "cov": "%s"}\n' \
    "$BUILD/bin/dfz" "$TOOLS/llvm-profdata" "$TOOLS/llvm-cov" > "$BUILD/coverage.json"
  echo "wrote $BUILD/coverage.json"
fi
echo "built $BUILD/bin/dfz"
