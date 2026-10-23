#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]: builds bin/gixfuzz against the gix sources in <src-dir>.
# With `coverage`, everything is instrumented for LLVM source-based coverage and
# <build-dir>/coverage.json names the binary and the matching llvm-profdata / llvm-cov.
set -euo pipefail
SRC=$(realpath "$1")
mkdir -p "$2"
BUILD=$(realpath "$2")
HERE=$(dirname "$(realpath "$0")")
MODE=${3:-}
EPHYRA=${EPHYRA:-$(cd "$(dirname "$0")/../../.." && pwd)}
REFGIT=${REFGIT:-/agent-run/work/ref/git-2.56.0/bin/git}
[ -x "$REFGIT" ] || REFGIT=/usr/bin/git

mkdir -p "$BUILD/crate" "$BUILD/bin/gitbin"
rsync -a --delete "$HERE/crate/src/" "$BUILD/crate/src/"
sed -e "s#@SRC@#$SRC#g" -e "s#@EPHYRA@#$EPHYRA#g" "$HERE/crate/Cargo.toml.in" > "$BUILD/crate/Cargo.toml.new"
cmp -s "$BUILD/crate/Cargo.toml.new" "$BUILD/crate/Cargo.toml" 2>/dev/null || mv "$BUILD/crate/Cargo.toml.new" "$BUILD/crate/Cargo.toml"
rm -f "$BUILD/crate/Cargo.toml.new"
# Resolve the same dependency versions as the target's workspace.
[ -f "$BUILD/crate/Cargo.lock" ] || cp "$SRC/Cargo.lock" "$BUILD/crate/Cargo.lock" 2>/dev/null || true

if [ "$MODE" = coverage ]; then
  # An explicit --target keeps build scripts and proc-macros uninstrumented (they would
  # otherwise drop .profraw files into the source tree).
  HOST=$(rustc -vV | sed -n 's/^host: //p')
  SYSROOT=$(rustc --print sysroot)
  RUSTFLAGS="${RUSTFLAGS:-} -C instrument-coverage" \
    cargo build --release --manifest-path "$BUILD/crate/Cargo.toml" --target-dir "$BUILD/target" --target "$HOST" 2>&1 | tail -n 30
  BIN="$BUILD/target/$HOST/release/gixfuzz"
  TOOLS="$SYSROOT/lib/rustlib/$HOST/bin"
  [ -x "$TOOLS/llvm-profdata" ] || rustup component add llvm-tools >/dev/null 2>&1 || true
  [ -x "$TOOLS/llvm-profdata" ] && [ -x "$TOOLS/llvm-cov" ] || { echo "llvm-tools missing in $TOOLS" >&2; exit 1; }
  cp "$BIN" "$BUILD/bin/gixfuzz.new" && mv "$BUILD/bin/gixfuzz.new" "$BUILD/bin/gixfuzz"
  printf '{"objects": ["%s"], "profdata": "%s", "cov": "%s"}\n' \
    "$BUILD/bin/gixfuzz" "$TOOLS/llvm-profdata" "$TOOLS/llvm-cov" > "$BUILD/coverage.json"
else
  cargo build --release --manifest-path "$BUILD/crate/Cargo.toml" --target-dir "$BUILD/target" 2>&1 | tail -n 30
  cp "$BUILD/target/release/gixfuzz" "$BUILD/bin/gixfuzz.new" && mv "$BUILD/bin/gixfuzz.new" "$BUILD/bin/gixfuzz"
fi
ln -sfn "$REFGIT" "$BUILD/bin/gitbin/git"
"$BUILD/bin/gitbin/git" --version
test -x "$BUILD/bin/gixfuzz"
