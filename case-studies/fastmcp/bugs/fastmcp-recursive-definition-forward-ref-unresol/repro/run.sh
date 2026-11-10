#!/bin/sh
# Usage: run.sh <python>   (a python where `import fastmcp` is the build to test)
# Exit 1 = bug reproduces, 0 = not, 2 = undecided.
PY=${1:-python}
cd "$(dirname "$0")" || exit 2
out=$("$PY" mre.py 2>&1); wrong=$("$PY" mre_wrong.py 2>&1)
if echo "$out" | grep -q 'not fully defined'; then echo "reproduced: $(echo "$out" | grep -o 'is not fully defined' | head -1)"; exit 1; fi
if [ "$wrong" = "Root(name='Bob', owner=None)" ]; then echo "reproduced: Pet.owner validated as Pet: $wrong"; exit 1; fi
if [ "$out" = "Root(node=None)" ] && echo "$wrong" | grep -q "pet=Root(name='Max'"; then echo "not reproduced"; exit 0; fi
echo "undecided"; echo "$out" | tail -2; echo "$wrong" | tail -2; exit 2
