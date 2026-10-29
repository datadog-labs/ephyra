#!/bin/bash
# usage: repro.sh <ocaml-install-prefix>   (the directory containing bin/ocaml)
# exit 1: the toplevel crashes (bug reproduces), 0: it does not, 2: cannot decide
P=$(realpath "$1"); cd "$(dirname "$0")"
[ -x "$P/bin/ocaml" ] && [ -x "$P/bin/ocamlc" ] || { echo "no $P/bin/ocaml"; exit 2; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cp poc.ml "$T"/ && cd "$T" || exit 2
echo "== ocamlc poc.ml -o poc.byte && ./poc.byte"
"$P/bin/ocamlc" -w -a poc.ml -o poc.byte && ./poc.byte || exit 2
echo "== ocaml poc.ml"
timeout 60 "$P/bin/ocaml" -w -a poc.ml > out.txt 2>&1; rc=$?
cat out.txt; echo "(exit $rc)"
[ $rc -eq 124 ] && exit 2
grep -q 'unbound at toplevel' out.txt && exit 1
exit 0
