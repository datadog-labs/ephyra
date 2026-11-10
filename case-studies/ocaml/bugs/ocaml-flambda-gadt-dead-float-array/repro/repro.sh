#!/bin/bash
# Usage: repro.sh <ocaml-install-prefix-or-bin-dir>
#   The directory must hold an flambda ocamlopt (configure --enable-flambda),
#   either in <dir>/bin or in <dir> itself.
# Exit 1: the bug reproduces (Fatal error "Bad definition for float array member").
# Exit 0: compiles and prints "ok". Exit 2: cannot decide.
D=$(realpath "$1")
[ -x "$D/bin/ocamlopt" ] && D="$D/bin"
OPT="$D/ocamlopt"
[ -x "$OPT" ] || { echo "no ocamlopt in $1"; exit 2; }
[ "$("$OPT" -config-var flambda)" = true ] || { echo "ocamlopt is not an flambda build"; exit 2; }
SRC="$(cd "$(dirname "$0")" && pwd)/gadt_float.ml"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cp "$SRC" "$T/"; cd "$T"
if [ -x "$D/ocamlc" ]; then
  "$D/ocamlc" gadt_float.ml -o bc.exe && echo "bytecode: $(./bc.exe)"
fi
timeout 300 "$OPT" gadt_float.ml -o nat.exe > log 2>&1; st=$?
cat log
if [ $st -eq 0 ]; then
  out=$(./nat.exe); echo "native: $out"
  [ "$out" = ok ] && exit 0 || exit 2
fi
grep -q 'Bad definition for float array member' log && exit 1
exit 2
