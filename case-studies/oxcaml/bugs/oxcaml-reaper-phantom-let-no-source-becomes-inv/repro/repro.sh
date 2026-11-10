#!/bin/bash
# usage: repro.sh <oxcaml-bin-dir>   (runs prog.ml with the failing and passing flag sets)
B=$1; T=$(mktemp -d); cp "$(dirname "$0")/prog.ml" "$T/"; cd "$T" || exit 2
for fl in "" "-O3 -g" "-O4" "-flambda2-reaper -g" "-O3 -g -flambda2-expert-phantom-lets" \
          "-O4 -g -flambda2-expert-phantom-lets" "-flambda2-reaper -g -flambda2-expert-phantom-lets" \
          "-O4 -g -gdwarf-may-alter-codegen-experimental"; do
  "$B/ocamlopt.opt" $fl prog.ml -o p.exe && ./p.exe; echo "# ocamlopt $fl: exit $?"
done
"$B/ocamlc.opt" prog.ml -o p.byte && ./p.byte; echo "# bytecode: exit $?"
