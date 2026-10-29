#!/bin/sh
# usage: ./run.sh [ty-binary]   (default: ty on PATH)
# Asks `ty server` (LSP) to rename method `meth` of the @final generic class C (repro.py line 6,
# at its declaration) to `renamed`, writes the result to renamed.py, then checks and runs it.
# exit 1 = the bug reproduces (c.meth() on line 11 was not renamed),
# 0 = both occurrences were renamed, 2 = could not run.
cd "$(dirname "$0")" || exit 2
ty=${1:-ty}
python3 lsp_rename.py repro.py 5 8 renamed --out renamed.py -- "$ty" server || exit 2
[ -s renamed.py ] || exit 2
cat renamed.py
"$ty" check --python-version 3.13 --output-format concise --no-progress renamed.py
python3 renamed.py
grep -q 'c\.meth()' renamed.py && exit 1
exit 0
