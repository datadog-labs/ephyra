#!/bin/bash
# Usage: repro.sh <ocaml-bin-dir>   (directory holding ocamlc, ocamlopt, ocamlrund)
# Expected (OCaml manual): the match fails, so Match_failure is raised:
#   Fatal error: exception Match_failure("m.ml", 1, 12)   (exit 2)
# Exit 1 if the bug reproduces (crash in bytecode or native), 0 if both
# programs raise Match_failure, 2 if it cannot decide.
BIN=$(realpath "$1")
SRC=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cp "$SRC/m.ml" "$T/"; cd "$T"
ulimit -c 0; export OCAMLRUNPARAM=v=0
"$BIN/ocamlc" -w -a m.ml -o m.byte || exit 2
"$BIN/ocamlopt" -w -a m.ml -o m.exe || exit 2
bug=0
for run in "./m.byte" "$BIN/ocamlrund ./m.byte" "./m.exe"; do
  timeout 10 $run > out.txt 2> err.txt; st=$?
  echo "$run: exit $st: $(cat out.txt err.txt | grep -v monitored | tail -1)"
  [ $st -eq 124 ] && exit 2
  if [ $st -ne 2 ] || ! grep -q 'Match_failure("m.ml", 1, 12)' err.txt; then bug=1; fi
done
exit $bug
