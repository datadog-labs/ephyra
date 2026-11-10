#!/bin/bash
# -use-ssa: a call followed only by an unused pure op is turned into a tail
# call by Ssa_tail_call, but the legacy Cfg_selectgen keeps call + return, so
# the default-on -ssa-validate (Cfg_compare) stops with a fatal error.
# usage: repro.sh <oxcaml-install-prefix-or-build-dir>
#   (directory holding bin/ocamlopt.opt, or install/bin/ocamlopt.opt)
# exit 1 = bug reproduces, 0 = does not, 2 = cannot decide
P=$(realpath "$1") || exit 2
if [ -x "$P/bin/ocamlopt.opt" ]; then I="$P/bin"; else I="$P/install/bin"; fi
[ -x "$I/ocamlopt.opt" ] || { echo "no ocamlopt.opt under $P"; exit 2; }
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cp "$HERE/prog.ml" "$T/" && cd "$T" || exit 2

echo "== bytecode (expected: 42)"
"$I/ocamlc.opt" prog.ml -o prog.byte && ./prog.byte; echo
echo "== ocamlopt -use-ssa -no-ssa-validate (expected: 42)"
"$I/ocamlopt.opt" -use-ssa -no-ssa-validate prog.ml -o prog.nv && ./prog.nv; echo
echo "== ocamlopt -use-ssa (validation on by default)"
timeout -k 2 120 "$I/ocamlopt.opt" -use-ssa prog.ml -o prog.ssa > c.err 2>&1; rc=$?
if [ $rc -eq 0 ]; then ./prog.ssa; echo; echo "compiled fine: not reproduced"; exit 0; fi
grep -a -A1 'Terminator mismatch' c.err
grep -a '^>> Fatal error' c.err
if grep -qF '>> Fatal error: CFG comparison MISMATCH for camlProg__f' c.err \
   && grep -qF 'tailcall "camlProg__g' c.err; then
  echo "reproduced (exit $rc)"; exit 1
fi
echo "failed differently (exit $rc):"; tail -5 c.err; exit 2
