#!/bin/bash
# ocamlopt -Oclassic -flambda2-expert-inline-effects-in-cmm aborts with
# "Fatal error: Duplicate order for bindings when flushing: to_cmm_split_N".
# usage: repro.sh <oxcaml-build-dir>   (expects <dir>/install/bin/ocamlopt.opt)
#        or set OCAMLBIN=/path/to/bin to point at an installed OxCaml bin directory.
# exit 1 = reproduces, 0 = does not, 2 = cannot decide
if [ -n "$OCAMLBIN" ]; then I=$OCAMLBIN; else B=$(realpath "$1") || exit 2; I="$B/install/bin"; fi
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
cp "$HERE/prog.ml" "$T/prog.ml"
cd "$T" || exit 2
"$I/ocamlc.opt" -c prog.ml >/dev/null 2>&1 || exit 2
timeout -k 2 120 "$I/ocamlopt.opt" -Oclassic -flambda2-expert-inline-effects-in-cmm -c prog.ml > c.err 2>&1; rc=$?
[ $rc -eq 0 ] && { echo "compiled fine: not reproduced"; exit 0; }
if grep -qF -- '>> Fatal error: Duplicate order for bindings when flushing: to_cmm_split_' c.err; then head -3 c.err; exit 1; fi
echo "failed differently:"; head -5 c.err; exit 2
