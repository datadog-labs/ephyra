#!/bin/bash
# usage: repro.sh <oxcaml-prefix-or-build-dir>
# Compiles prog.ml with several flag sets and runs it; bytecode for reference.
# exit 1 = bug reproduces (-use-ssa compile aborts with the CFG comparison fatal error), 0 = not, 2 = cannot decide
P=$(realpath "$1") || exit 2
if [ -x "$P/bin/ocamlopt.opt" ]; then I="$P/bin"; elif [ -x "$P/install/bin/ocamlopt.opt" ]; then I="$P/install/bin"; else exit 2; fi
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cp "$HERE/prog.ml" "$T/prog.ml"; cd "$T" || exit 2
bug=0
for fl in "" "-use-ssa" "-use-ssa -O3" "-use-ssa -Oclassic" "-experimental-optimizations" "-use-ssa -no-ssa-validate"; do
  rm -f p.exe; "$I/ocamlopt.opt" $fl prog.ml -o p.exe >err.txt 2>&1; rc=$?
  if [ -x p.exe ]; then out=$(./p.exe | tr '\n' ' '); else out=-; fi
  f=$(grep -a -m1 'Fatal error: CFG comparison MISMATCH' err.txt)
  [ -n "$f" ] && bug=1
  echo "[$fl] compile exit $rc ${f:+($f)} output: $out"
done
"$I/ocamlc.opt" prog.ml -o b.exe && echo "[bytecode] output: $(./b.exe | tr '\n' ' ')"
exit $bug
