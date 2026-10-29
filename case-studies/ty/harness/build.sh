#!/usr/bin/env bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]  (ephyra-agent harness contract)
#
# <src-dir> is an astral-sh/ruff checkout (plus the loop's patches). Builds into <build-dir>:
#   bin/ty   ty from <src-dir>: release, no LTO, debug assertions and overflow checks on
#            (with "coverage": -C instrument-coverage instead, plus <build-dir>/coverage.json)
#   bin/tyz  this harness (never instrumented; it only drives ty and CPython)
#   bin/py   a CPython 3.13+ (the generated programs use TypeIs, ReadOnly, PEP 695/696)
# Incremental and re-runnable. Needs the network on first run (ruff's pinned toolchain).
set -euo pipefail
SRC="$(cd "$1" && pwd)"
mkdir -p "$2"
BUILD="$(cd "$2" && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"
MODE="${3:-}"

export CARGO_HOME="${CARGO_HOME:-/agent-run/work/.cargo}"
[ -d "$CARGO_HOME" ] || unset CARGO_HOME
EPHYRA="${EPHYRA:-$(cd "$(dirname "$0")/../../.." && pwd)}"
[ -d "$EPHYRA/crates/ephyra" ] || EPHYRA=/agent-run/ephyra
[ -d "$EPHYRA/crates/ephyra" ] || { echo "ephyra not found (set EPHYRA to a checkout with crates/ephyra)" >&2; exit 1; }
mkdir -p "$BUILD/bin"

# --- CPython 3.13+ -------------------------------------------------------------------------------
# TYZ_PY, then a python3.13 installed at setup time under work/.python (uv), then PATH.
PY="${TYZ_PY:-}"
if [ -z "$PY" ]; then
  for c in /agent-run/work/.python/cpython-3.13*/bin/python3.13 "$(command -v python3.13 || true)"; do
    [ -n "$c" ] && [ -x "$c" ] && { PY="$c"; break; }
  done
fi
[ -n "$PY" ] || { echo "no python3.13: set TYZ_PY or install one under work/.python (uv python install 3.13)" >&2; exit 1; }
"$PY" -c 'import sys; assert sys.version_info >= (3, 13), sys.version' || { echo "$PY is older than 3.13" >&2; exit 1; }
# A virtual environment with pydantic (the generator's pydantic models; ty's dedicated support
# finds it through VIRTUAL_ENV, which tyz sets to <build>/venv). bin/py runs its python: a symlink
# would not find the venv's pyvenv.cfg. Public PyPI, pinned to the versions of ty's mdtests.
PYD_REQ="pydantic==2.13.4 pydantic-settings==2.14.2"
if ! [ -f "$BUILD/venv/.req" ] || [ "$(cat "$BUILD/venv/.req")" != "$PYD_REQ" ] || ! "$BUILD/venv/bin/python" -I -c 'import pydantic' 2> /dev/null; then
  rm -rf "$BUILD/venv"
  "$PY" -m venv "$BUILD/venv"
  # shellcheck disable=SC2086
  "$BUILD/venv/bin/python" -m pip install -q --isolated --disable-pip-version-check --no-cache-dir \
    --index-url "${TYZ_PIP_INDEX:-https://pypi.org/simple}" $PYD_REQ
  echo "$PYD_REQ" > "$BUILD/venv/.req"
fi
rm -f "$BUILD/bin/py"
printf '#!/bin/sh\nexec "%s" "$@"\n' "$BUILD/venv/bin/python" > "$BUILD/bin/py.new"
chmod +x "$BUILD/bin/py.new"
mv "$BUILD/bin/py.new" "$BUILD/bin/py"

# --- ty ------------------------------------------------------------------------------------------
# cargo runs from <src-dir> so rustup picks ruff's rust-toolchain.toml. If that toolchain cannot be
# installed or is broken (the loop's RUSTUP_HOME is read-only), use the default stable toolchain.
if [ -z "${RUSTUP_TOOLCHAIN:-}" ] && ! (cd "$SRC" && rustc -V > /dev/null 2>&1); then
  echo "ruff's pinned toolchain is unusable here: using stable" >&2
  export RUSTUP_TOOLCHAIN=stable
fi
HOST="$(cd "$SRC" && rustc -vV | sed -n 's/^host: //p')"
export CARGO_PROFILE_RELEASE_LTO=false
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
if [ "$MODE" = coverage ]; then
  (cd "$SRC" && rustup component add llvm-tools > /dev/null 2>&1 || true)
  SYSROOT="$(cd "$SRC" && rustc --print sysroot)"
  TOOLS="$SYSROOT/lib/rustlib/$HOST/bin"
  [ -x "$TOOLS/llvm-profdata" ] && [ -x "$TOOLS/llvm-cov" ] || { echo "llvm-tools missing in $TOOLS" >&2; exit 1; }
  # An explicit --target keeps instrumentation off build scripts and proc macros.
  (cd "$SRC" && RUSTFLAGS="${RUSTFLAGS:-} -C instrument-coverage" LLVM_PROFILE_FILE="$BUILD/profraw-build/%m_%p.profraw" \
    cargo build --release -p ty --target "$HOST" --target-dir "$BUILD/target-ty-cov" 2>&1 | tail -n 20)
  cp "$BUILD/target-ty-cov/$HOST/release/ty" "$BUILD/bin/ty.new"
  mv "$BUILD/bin/ty.new" "$BUILD/bin/ty"
  printf '{"objects": ["%s"], "profdata": "%s", "cov": "%s"}\n' \
    "$BUILD/bin/ty" "$TOOLS/llvm-profdata" "$TOOLS/llvm-cov" > "$BUILD/coverage.json"
  echo "wrote $BUILD/coverage.json"
else
  (cd "$SRC" && CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true \
    cargo build --release -p ty --target-dir "$BUILD/target-ty" 2>&1 | tail -n 20)
  cp "$BUILD/target-ty/release/ty" "$BUILD/bin/ty.new"
  mv "$BUILD/bin/ty.new" "$BUILD/bin/ty"
fi
"$BUILD/bin/ty" --version

# --- tyinc (incremental vs fresh checks, in-process against <src-dir>'s ty_project) -------------
# Its own workspace with ruff's Cargo.lock, built with ruff's toolchain and the same flags as bin/ty.
ICRATE="$BUILD/tyinc-crate"
mkdir -p "$ICRATE/src"
cmp -s "$HERE/tyinc/src/main.rs" "$ICRATE/src/main.rs" || cp "$HERE/tyinc/src/main.rs" "$ICRATE/src/"
sed -e "s#@SRC@#$SRC#g" "$HERE/tyinc/Cargo.toml.in" > "$ICRATE/Cargo.toml.new"
cmp -s "$ICRATE/Cargo.toml.new" "$ICRATE/Cargo.toml" || mv "$ICRATE/Cargo.toml.new" "$ICRATE/Cargo.toml"
rm -f "$ICRATE/Cargo.toml.new"
[ -f "$ICRATE/Cargo.lock" ] || cp "$SRC/Cargo.lock" "$ICRATE/Cargo.lock"
TC="${RUSTUP_TOOLCHAIN:-$(cd "$SRC" && rustup show active-toolchain | awk '{print $1}')}"
if [ "$MODE" = coverage ]; then
  (cd "$ICRATE" && RUSTUP_TOOLCHAIN="$TC" RUSTFLAGS="${RUSTFLAGS:-} -C instrument-coverage" LLVM_PROFILE_FILE="$BUILD/profraw-build/%m_%p.profraw" \
    cargo build --release --target "$HOST" --target-dir "$BUILD/target-tyinc-cov" 2>&1 | tail -n 20)
  cp "$BUILD/target-tyinc-cov/$HOST/release/tyinc" "$BUILD/bin/tyinc.new"
  printf '{"objects": ["%s", "%s"], "profdata": "%s", "cov": "%s"}\n' \
    "$BUILD/bin/ty" "$BUILD/bin/tyinc" "$TOOLS/llvm-profdata" "$TOOLS/llvm-cov" > "$BUILD/coverage.json"
else
  (cd "$ICRATE" && RUSTUP_TOOLCHAIN="$TC" cargo build --release --target-dir "$BUILD/target-tyinc" 2>&1 | tail -n 20)
  cp "$BUILD/target-tyinc/release/tyinc" "$BUILD/bin/tyinc.new"
fi
mv "$BUILD/bin/tyinc.new" "$BUILD/bin/tyinc"

# --- tyz -----------------------------------------------------------------------------------------
CRATE="$BUILD/crate"
mkdir -p "$CRATE/src" "$CRATE/py"
# copy sources only when changed so cargo's fingerprints stay valid
for f in "$HERE"/src/*.rs; do cmp -s "$f" "$CRATE/src/$(basename "$f")" || cp "$f" "$CRATE/src/"; done
for f in "$HERE"/py/*.py; do cmp -s "$f" "$CRATE/py/$(basename "$f")" || cp "$f" "$CRATE/py/"; done
cmp -s "$HERE/known.json" "$CRATE/known.json" || cp "$HERE/known.json" "$CRATE/known.json"
[ -f "$CRATE/Cargo.lock" ] || cp "$HERE/Cargo.lock" "$CRATE/Cargo.lock"
sed -e "s#^ephyra = .*#ephyra = { path = \"$EPHYRA/crates/ephyra\", default-features = false }#" "$HERE/Cargo.toml" > "$CRATE/Cargo.toml.new"
cmp -s "$CRATE/Cargo.toml.new" "$CRATE/Cargo.toml" || mv "$CRATE/Cargo.toml.new" "$CRATE/Cargo.toml"
rm -f "$CRATE/Cargo.toml.new"
(cd "$CRATE" && cargo build --release --target-dir "$BUILD/target-tyz" 2>&1 | tail -n 20)
cp "$BUILD/target-tyz/release/tyz" "$BUILD/bin/tyz.new"
mv "$BUILD/bin/tyz.new" "$BUILD/bin/tyz"
echo "built $BUILD/bin/{ty,tyz,py}${MODE:+ ($MODE)}"
