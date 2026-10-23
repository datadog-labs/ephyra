#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build.sh <src-dir> <build-dir> [coverage]
# Normal: copies src into <build>/src, configures with flambda invariants, make install
#   into <build>/install. Incremental (rsync, dune).
# coverage: same normal build, plus a second tree <build>/covsrc configured with
#   --enable-coverage where only the boot compiler (same sources, built by OCaml 5.4)
#   is built, instrumented with bisect_ppx. run.sh uses that compiler for the native
#   compiles when <build>/covsrc exists. Writes <build>/coverage.json.
set -euo pipefail
SRC=$(realpath "$1"); mkdir -p "$2"; B=$(realpath "$2"); MODE=${3:-}
H=$(dirname "$(realpath "$0")")
WORK=/agent-run/work
export PATH=$WORK/tools/boot/bin:$PATH
export DUNE_CACHE=disabled
J=${BUILD_JOBS:-16}

sync_tree() { # <dst>
  mkdir -p "$1"
  # no --delete: configure and make leave generated files in the tree
  rsync -a --checksum --exclude=/_build --exclude=/.git \
    --exclude=/Makefile.config --exclude=/config.status --exclude=/config.log \
    --exclude=/configure --exclude=/autom4te.cache \
    "$SRC"/ "$1"/
}

# --- harness (generator + runner) ---
( cd "$H/oxfuzz" && CARGO_TARGET_DIR="$B/harness-target" cargo build --release -q )

# --- normal build ---
sync_tree "$B/src"
cd "$B/src"
if [ ! -f "$B/src/Makefile.config" ] || [ ! -f "$B/.configured" ]; then
  autoconf
  ./configure --prefix="$B/install" --enable-flambda-invariants
  touch "$B/.configured"
fi
make -j"$J" install
test -x "$B/install/bin/ocamlopt.opt"
test -x "$B/install/bin/ocamlc.opt"

if [ "$MODE" = coverage ]; then
  COVT=$WORK/tools/cov
  export PATH=$COVT/bin:$PATH
  export OCAMLPATH=$COVT/lib${OCAMLPATH:+:$OCAMLPATH}
  sync_tree "$B/covsrc"
  cd "$B/covsrc"
  if [ ! -f "$B/covsrc/Makefile.config" ] || [ ! -f "$B/.cov-configured" ]; then
    autoconf
    ./configure --prefix="$B/covinstall" --enable-flambda-invariants --enable-coverage
    touch "$B/.cov-configured"
  fi
  # bisect_ppx instrumentation trips warnings that the dev profile makes
  # errors (71 unused-tmc-attribute, 51 expected tailcall); the boot context
  # pins OCAMLPARAM in dune-workspace, so turn warn-error off there.
  sed -i 's/("OCAMLPARAM" "")/("OCAMLPARAM" "_,warn-error=-a")/' "$B/covsrc/dune-workspace"
  # absolute source paths in the .coverage files: read by the patched bisect
  # ppx (tools/bisect-abs-path.py), so cobertura works from any directory
  grep -q BISECT_SOURCE_ROOT "$B/covsrc/dune-workspace" ||
    sed -i "s|(\"OCAMLPARAM\" \"_,warn-error=-a\")|(\"OCAMLPARAM\" \"_,warn-error=-a\") (\"BISECT_SOURCE_ROOT\" \"$B/covsrc\")|" "$B/covsrc/dune-workspace"
  # the bisect runtime links the system unix library; the root dune file
  # also passes unix.cmx?a by hand ("Duplicated implementations").
  sed -i 's/(ocamlopt_flags (:standard -I +unix unix.cmxa))/(ocamlopt_flags (:standard -I +unix))/; s/(ocamlc_flags (:standard -I +unix unix.cma))/(ocamlc_flags (:standard -I +unix))/' "$B/covsrc/dune"
  make -j"$J" boot-compiler

  python3 - "$B" "$COVT/bin/bisect-ppx-report" <<'PY'
import json, sys, os
b, rep = sys.argv[1], sys.argv[2]
objs = [os.path.join(b, "covsrc/_build/default/boot_ocamlopt.exe")]
json.dump({"kind": "bisect", "report": rep, "objects": objs},
          open(os.path.join(b, "coverage.json"), "w"), indent=1)
PY
fi
echo "build.sh: done ($MODE)"
