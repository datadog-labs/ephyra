#!/bin/bash
# usage: repro.sh <oxcaml-prefix-or-build-dir>
#   accepts an install prefix (<p>/bin/ocamlopt.opt) or a harness build dir (<b>/install/bin/ocamlopt.opt)
# exit 1 = bug reproduces, 0 = does not, 2 = cannot decide
P=$(realpath "$1") || exit 2
if [ -x "$P/bin/ocamlopt.opt" ]; then I="$P/bin"; elif [ -x "$P/install/bin/ocamlopt.opt" ]; then I="$P/install/bin"; else exit 2; fi
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cp "$HERE/prog.ml" "$T/prog.ml"; cd "$T" || exit 2
"$I/ocamlopt.opt" -c prog.ml >/dev/null 2>&1 || { echo "default compile failed"; exit 2; }
timeout -k 2 120 "$I/ocamlopt.opt" -use-ssa -c prog.ml > err.txt 2>&1; rc=$?
if [ $rc -eq 0 ]; then echo "-use-ssa: compiled fine (not reproduced)"; exit 0; fi
if grep -q 'Fatal error: CFG comparison MISMATCH' err.txt; then
  grep -a -m1 'Terminator mismatch' err.txt; grep -a -m1 'Fatal error' err.txt; exit 1
fi
tail -5 err.txt; exit 2
