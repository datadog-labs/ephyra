#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# make-seeds.sh <src-dir> <wasm-opt> <out-dir>
# Builds the --initial-fuzz corpus: binaryen's lit tests that parse and
# validate under the feature set the harness uses, as binaries.
# Skips the files scripts/test/fuzzing.py lists as unfuzzable and *.fail.*.
set -u
SRC=$(realpath "$1"); OPT=$(realpath "$2"); OUT=$3
mkdir -p "$OUT"
OUT=$(realpath "$OUT")
F="-all --disable-strings --disable-fp16 --disable-custom-descriptors --disable-multibyte --disable-compact-imports --disable-relaxed-atomics --disable-acquire-release-atomics --disable-shared-everything --disable-relaxed-simd"
UNFUZZ=$(python3 -I -c "
import ast,sys
src=open(sys.argv[1]).read()
t=ast.parse(src)
for n in t.body:
    if isinstance(n, ast.Assign) and getattr(n.targets[0],'id','')=='unfuzzable':
        print('\n'.join(ast.literal_eval(n.value)))
" "$SRC/scripts/test/fuzzing.py")
LIST=$(mktemp)
find "$SRC/test/lit" -name '*.wast' | sort | while read -r f; do
  b=$(basename "$f")
  case "$b" in *.fail.*) continue;; esac
  grep -qxF "$b" <<< "$UNFUZZ" && continue
  # Multi-module files are not handled.
  [ "$(grep -c '^ *(module' "$f")" -gt 1 ] && continue
  echo "$f"
done > "$LIST"
export OPT OUT F SRC
xargs -P "${JOBS:-14}" -I{} bash -c '
  f={}; n=$(echo "${f#$SRC/test/lit/}" | tr "/" "_"); n=${n%.wast}
  timeout 20 "$OPT" "$f" $F -o "$OUT/$n.wasm" >/dev/null 2>&1 || rm -f "$OUT/$n.wasm"
' < "$LIST"
rm -f "$LIST"
# Keep small seeds only (fuzz_opt avoids large initial contents too).
find "$OUT" -name '*.wasm' -size +32k -delete
echo "seeds: $(ls "$OUT" | wc -l)"
