#!/bin/bash
# usage: repro.sh <build-dir>   (expects <build-dir>/install/bin/ocamlopt.opt)
#   or:  OCAMLOPT=/path/to/ocamlopt.opt repro.sh
# exit 1 = bug reproduces (ocamlopt crashes), 0 = it does not, 2 = cannot decide
HERE=$(cd "$(dirname "$0")" && pwd)
if [ -n "$1" ]; then OPT=$(realpath "$1")/install/bin/ocamlopt.opt; else OPT=${OCAMLOPT:-ocamlopt.opt}; fi
command -v "$OPT" >/dev/null 2>&1 || { echo "no ocamlopt at $OPT"; exit 2; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cp "$HERE/prog.ml" "$T/" && cd "$T" || exit 2
OCAMLRUNPARAM=b timeout 240 "$OPT" -Oclassic -flambda2-expert-inline-effects-in-cmm \
  prog.ml -o prog > err.txt 2>&1; rc=$?
if [ $rc -eq 0 ]; then
  out=$(./prog); echo "compiled; output: $out (expected 41)"
  [ "$out" = 41 ] && exit 0 || exit 2
fi
head -4 err.txt
grep -qF 'Invalid_argument("Array.iter2' err.txt && exit 1
exit 2
