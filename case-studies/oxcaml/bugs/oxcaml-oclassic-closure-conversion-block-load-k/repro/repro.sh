#!/bin/bash
# usage: repro.sh <build-dir>   (compilers in <build-dir>/install/bin)
# exit 1 = ocamlopt -Oclassic dies with "Illegal move between registers", 0 = it does not, 2 = cannot decide
B=$(realpath "$1") || exit 2
HERE=$(cd "$(dirname "$0")" && pwd)
I="$B/install/bin"
L=(-I +stdlib_upstream_compatible)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
cp "$HERE/prog.ml" "$T/" && cd "$T" || exit 2
"$I/ocamlc.opt" "${L[@]}" stdlib_upstream_compatible.cma prog.ml -o p.byte >/dev/null 2>&1 || exit 2
echo "bytecode: $("$I/ocamlrun" ./p.byte)"
"$I/ocamlopt.opt" -Oclassic "${L[@]}" stdlib_upstream_compatible.cmxa prog.ml -o p.exe > c.err 2>&1
rc=$?
if [ $rc -eq 0 ]; then echo "-Oclassic: $(./p.exe)"; exit 0; fi
head -1 c.err
grep -qF 'Fatal error: Illegal move between registers of differing types' c.err && exit 1
exit 2
