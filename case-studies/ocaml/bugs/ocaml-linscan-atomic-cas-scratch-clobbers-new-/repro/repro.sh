#!/bin/bash
# Atomic.compare_and_set returns true but does not store the new value
# when compiled with ocamlopt -linscan (amd64).
# usage: repro.sh <bin-dir of an OCaml install, e.g. $PREFIX/bin>
# exit 1: bug reproduces, 0: does not, 2: cannot decide
set -u
BIN=$(realpath "${1:?usage: repro.sh <ocaml-bin-dir>}")
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cp "$HERE/cas.ml" "$T"/; cd "$T"
"$BIN/ocamlc" cas.ml -o cas.byte || exit 2
"$BIN/ocamlopt" -linscan cas.ml -o cas.exe || exit 2
exp=$("$BIN/ocamlrun" ./cas.byte) || exit 2
act=$(./cas.exe) || exit 2
echo "bytecode:          $exp"
echo "ocamlopt -linscan: $act"
[ "$exp" = "$act" ] && exit 0
exit 1
