#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]
# Builds two OCaml compilers from <src-dir> (closure and flambda middle ends) into
# <build-dir>/{closure,flambda}/inst, and the Rust driver into <build-dir>/bin/ocfuzz.
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

# Independent reference for the ref oracle: the OCaml 5.4.0 release, bytecode
# only (front end + Lambda of a year-older compiler), built once into
# <build-dir>/ref54/inst. Not built from <src-dir>: patches do not apply to it.
# 5.4.0 shares some bugs that the loop already fixed in trunk; ref54-patches/
# backports those fixes so the ref oracle does not keep reporting the
# reference's copy of a known bug as a trunk/5.4.0 difference. The stamp
# records which backports the installed reference has.
build_ref54() {
  local d=$B/ref54
  local stamp
  stamp=$(cat "$H"/ref54-patches/*.diff | sha256sum | cut -c1-16)
  [ -x "$d/inst/bin/ocamlc" ] && [ -x "$d/inst/bin/ocamlrun" ] \
    && [ "$(cat "$d/inst/ref54-patches.stamp" 2>/dev/null)" = "$stamp" ] && return 0
  rm -rf "$d"; mkdir -p "$d"
  cd "$d"
  local tb=$B/.cache/ocaml-5.4.0.tar.gz
  if [ ! -s "$tb" ]; then
    mkdir -p "$B/.cache"
    curl -sSfL -o "$tb.tmp" https://github.com/ocaml/ocaml/archive/refs/tags/5.4.0.tar.gz
    mv "$tb.tmp" "$tb"
  fi
  tar xzf "$tb"
  cd ocaml-5.4.0
  local p
  for p in "$H"/ref54-patches/*.diff; do
    patch -p1 -s < "$p" || { echo "ref54: $p does not apply" >&2; return 1; }
  done
  ./configure --prefix="$d/inst" --disable-native-compiler --disable-ocamldoc --disable-debugger \
    --disable-ocamltest > ../configure.log 2>&1
  make -j16 > ../make.log 2>&1 || { tail -30 ../make.log >&2; return 1; }
  make install > ../install.log 2>&1 || { tail -30 ../install.log >&2; return 1; }
  echo "$stamp" > "$d/inst/ref54-patches.stamp"
  cd "$d" && rm -rf ocaml-5.4.0
}
build_ref54

# Rust driver (generator + oracle)
cd "$H/ocfuzz"
CARGO_TARGET_DIR="$B/cargo-target" cargo build --release -q
mkdir -p "$B/bin"
cp "$B/cargo-target/release/ocfuzz" "$B/bin/ocfuzz"
echo "build ok: $B"
