#!/bin/bash
# ocamlopt -Oclassic crashes with Invalid_argument("Array.iter2: ...") on an
# integer division by a literal 0 inside a try whose handler reads a ref.
# usage: repro.sh <oxcaml-build-dir-or-install-prefix>
# exit 1 = reproduces, 0 = does not, 2 = cannot decide
P=$(realpath "$1") || exit 2
if [ -x "$P/install/bin/ocamlopt.opt" ]; then I="$P/install/bin"; else I="$P/bin"; fi
[ -x "$I/ocamlopt.opt" ] || { echo "no ocamlopt.opt under $P"; exit 2; }
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cp "$HERE/prog.ml" "$T/"; cd "$T" || exit 2
"$I/ocamlc.opt" prog.ml -o p.byte || exit 2
echo "bytecode output: $(./p.byte)"
timeout -k 2 240 "$I/ocamlopt.opt" -Oclassic prog.ml -o p.nat > c.err 2>&1; rc=$?
if [ $rc -eq 0 ]; then echo "-Oclassic output: $(./p.nat)"; exit 0; fi
cat c.err
grep -qF 'Fatal error: exception Invalid_argument("Array.iter2' c.err && exit 1
exit 2
