#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# ref54.sh <build-dir>
# Independent reference for the ref oracle: the OCaml 5.4.0 release, bytecode
# only (front end + Lambda of a year-older compiler), built once into
# <build-dir>/ref54/inst. Not built from the target's src: patches/ do not apply to it.
# 5.4.0 shares some bugs that trunk has fixed; ref54-patches/ backports those
# fixes so the ref oracle does not keep reporting the reference's copy of a
# known bug. The stamp records which backports the installed reference has.
set -euo pipefail
B=$(realpath "$1")
H=$(dirname "$(realpath "$0")")
d=$B/ref54
stamp=$(cat "$H"/ref54-patches/*.diff | sha256sum | cut -c1-16)
[ -x "$d/inst/bin/ocamlc" ] && [ -x "$d/inst/bin/ocamlrun" ] \
  && [ "$(cat "$d/inst/ref54-patches.stamp" 2>/dev/null)" = "$stamp" ] && exit 0
rm -rf "$d"; mkdir -p "$d"
cd "$d"
tb=$B/.cache/ocaml-5.4.0.tar.gz
if [ ! -s "$tb" ]; then
  mkdir -p "$B/.cache"
  curl -sSfL -o "$tb.tmp" https://github.com/ocaml/ocaml/archive/refs/tags/5.4.0.tar.gz
  mv "$tb.tmp" "$tb"
fi
tar xzf "$tb"
cd ocaml-5.4.0
for p in "$H"/ref54-patches/*.diff; do
  patch -p1 -s < "$p" || { echo "ref54: $p does not apply" >&2; exit 1; }
done
./configure --prefix="$d/inst" --disable-native-compiler --disable-ocamldoc --disable-debugger \
  --disable-ocamltest > ../configure.log 2>&1
make -j16 > ../make.log 2>&1 || { tail -30 ../make.log >&2; exit 1; }
make install > ../install.log 2>&1 || { tail -30 ../install.log >&2; exit 1; }
echo "$stamp" > "$d/inst/ref54-patches.stamp"
cd "$d" && rm -rf ocaml-5.4.0
