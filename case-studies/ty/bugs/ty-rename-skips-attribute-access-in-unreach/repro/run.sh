#!/bin/sh
# usage: ./run.sh [ty-binary]   (default: ty on PATH)
# Asks `ty server` (LSP) to rename attribute `x` of class C (repro.py line 5, at its declaration)
# to `y`, writes the result to renamed.py, then checks renamed.py for win32.
# exit 1 = the bug reproduces (c.x inside the `sys.platform == "win32"` branch was not renamed),
# 0 = all three occurrences were renamed, 2 = could not run.
cd "$(dirname "$0")" || exit 2
ty=${1:-ty}
python3 lsp_rename.py repro.py 4 4 y --out renamed.py -- "$ty" server || exit 2
cat renamed.py
"$ty" check --python-version 3.13 --python-platform win32 --output-format concise --no-progress renamed.py
grep -q 'c\.x' renamed.py && exit 1
exit 0
