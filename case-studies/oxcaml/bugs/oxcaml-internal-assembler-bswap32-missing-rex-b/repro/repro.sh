#!/bin/bash
# usage: repro.sh <oxcaml-install-prefix>   (directory containing bin/ocamlopt.opt)
# exit 1 = bug reproduces, 0 = does not, 2 = cannot decide
P=$(realpath "$1") || exit 2
BIN="$P/bin"; [ -x "$BIN/ocamlopt.opt" ] || BIN="$P/install/bin"
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cp "$HERE/bswap.ml" "$T/" && cd "$T" || exit 2
"$BIN/ocamlc.opt" bswap.ml -o bswap.byte || exit 2
"$BIN/ocamlopt.opt" bswap.ml -o bswap.ext || exit 2
"$BIN/ocamlopt.opt" -internal-assembler bswap.ml -o bswap.int || exit 2
exp=$(./bswap.byte); ext=$(./bswap.ext)
act=$(./bswap.int 2>&1); rc=$?
echo "bytecode:             $exp"
echo "native (system as):   $ext"
echo "native (-internal-assembler): $act (exit $rc)"
if command -v objdump >/dev/null; then
  mkdir ext int && cp bswap.ml ext/ && cp bswap.ml int/
  (cd ext && "$BIN/ocamlopt.opt" -c bswap.ml)
  (cd int && "$BIN/ocamlopt.opt" -internal-assembler -c bswap.ml)
  echo "--- bswap instructions, system assembler:"
  objdump -d ext/bswap.o | grep -P '	bswap'
  echo "--- bswap instructions, -internal-assembler:"
  objdump -d int/bswap.o | grep -P '	bswap'
fi
[ "$exp" = "$ext" ] || exit 2
[ "$rc" = 0 ] && [ "$act" = "$exp" ] && exit 0
exit 1
