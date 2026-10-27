#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]
# Builds two OCaml compilers from <src-dir> (closure and flambda middle ends) into
# <build-dir>/{closure,flambda}/inst, and the ephyra-based Rust driver into <build-dir>/bin/ocfuzz.
# With "coverage", the compilers' lambda/middle_end/asmcomp are instrumented with
# bisect_ppx (cov/build-cov.sh) and <build-dir>/coverage.json is written.
set -euo pipefail
SRC=$(realpath "$1")
mkdir -p "$2"
B=$(realpath "$2")
MODE=${3:-}
H=$(dirname "$(realpath "$0")")

build_cfg() { # cfg configure-args...
  local cfg=$1; shift
  local d=$B/$cfg
  mkdir -p "$d/src"
  rsync -a --exclude .git "$SRC/" "$d/src/"
  cd "$d/src"
  local want="$*"
  if [ ! -f Makefile.config ] || [ "$(cat ../configure.args 2>/dev/null)" != "$want" ]; then
    ./configure --prefix="$d/inst" "$@" > ../configure.log 2>&1
    echo "$want" > ../configure.args
  fi
  make -j16 > ../make.log 2>&1 || { tail -30 ../make.log >&2; return 1; }
  make install > ../install.log 2>&1 || { tail -30 ../install.log >&2; return 1; }
}

if [ "$MODE" = coverage ]; then
  bash "$H/cov/build-cov.sh" "$SRC" "$B" closure > "$B/cov-closure.log" 2>&1 &
  p1=$!
  bash "$H/cov/build-cov.sh" "$SRC" "$B" flambda > "$B/cov-flambda.log" 2>&1 &
  p2=$!
  wait $p1 || { tail -30 "$B/cov-closure.log" >&2; exit 1; }
  wait $p2 || { tail -30 "$B/cov-flambda.log" >&2; exit 1; }
  # the wrapper runs the report from / so cobertura keeps absolute source paths
  cp "$H/cov/report.sh" "$B/tools/bin/bisect-ppx-report-abs"
  chmod +x "$B/tools/bin/bisect-ppx-report-abs"
  cat > "$B/coverage.json" <<EOF
{"kind": "bisect", "report": "$B/tools/bin/bisect-ppx-report-abs"}
EOF
else
  build_cfg closure &
  p1=$!
  build_cfg flambda --enable-flambda --enable-flambda-invariants &
  p2=$!
  wait $p1
  wait $p2
fi

# OCaml 5.4.0 release bytecode compiler, the ref oracle's reference (same in coverage mode)
bash "$H/ref54.sh" "$B"

# Rust driver (generator + oracle)
cd "$H/ocfuzz"
CARGO_TARGET_DIR="$B/cargo-target" cargo build --release -q
mkdir -p "$B/bin"
cp "$B/cargo-target/release/ocfuzz" "$B/bin/ocfuzz"
echo "build ok: $B"
