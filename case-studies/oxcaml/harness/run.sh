#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# run.sh <build-dir> <seed> <seconds> <out-dir>
# One search process: generate OCaml programs, compare native (flambda2 flag
# matrix) against bytecode, record crashes of the native compiler.
set -u
B=$(realpath "$1"); SEED=$2; SECS=$3; OUT=$4
mkdir -p "$OUT"; OUT=$(realpath "$OUT")
H=$(dirname "$(realpath "$0")")
BIN="$B/harness-target/release/oxfuzz"
[ -x "$BIN" ] || BIN="$H/oxfuzz/target/release/oxfuzz"
# coverage build: the native compiler is the bisect_ppx-instrumented boot
# compiler built from the same sources; it reads the installed stdlib.
COV="$B/covsrc/_build/default/boot_ocamlopt.exe"
if [ -x "$COV" ]; then
  export OXFUZZ_OCAMLOPT="$COV"
  export OCAMLLIB="$B/install/lib/ocaml"
  export OXFUZZ_BISECT_REPORT=/agent-run/work/tools/cov/bin/bisect-ppx-report
fi
# Private scratch dir. Not "$$": runs in separate PID namespaces share pids,
# and two searches compiling into the same directory corrupt each other.
# Inside the (unique) out dir rather than TMPDIR, which may be shared or noexec.
W=$(mktemp -d "$OUT/scratch.XXXXXXXX")
export OXFUZZ_WORK="$W/case"
"$BIN" run "$B" "$SEED" "$SECS" "$OUT" &
child=$!
trap 'kill $child 2>/dev/null; wait $child; rm -rf "$W"; exit 143' TERM INT
wait $child
rc=$?
rm -rf "$W"
exit $rc
