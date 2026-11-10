#!/bin/bash
# Usage: repro.sh <ocaml-bin-dir>   (directory holding ocamlc, ocamlrun, ocamlopt of an amd64 build)
# Exit 1: bug reproduces (native output differs from bytecode), 0: does not, 2: cannot decide.
# Atomic.exchange stores a stale register on amd64 when the result's pseudo-register is spilled.
set -u
BIN=$(realpath "$1")
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cp "$HERE/poc-loop.ml" "$T/"; cd "$T"
"$BIN/ocamlc" -w -a poc-loop.ml -o poc.byte || exit 2
"$BIN/ocamlopt" -w -a poc-loop.ml -o poc.exe || exit 2
exp=$("$BIN/ocamlrun" ./poc.byte) || exit 2
act=$(./poc.exe) || exit 2
echo "bytecode: $exp"
echo "native:   $act"
[ "$exp" = 1 ] || exit 2
[ "$act" = "$exp" ] && exit 0
exit 1
