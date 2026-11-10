#!/bin/sh
# usage: run.sh <path-to-ty>
# Starts `<ty> server`, requests the inlay hints of repro.py, applies their text edits (what an
# editor does on double-click) and runs `<ty> check` on the result.
# exit 1: the accepted hint produces an invalid-attribute-override error (bug present); 0: it does not.
here=$(cd "$(dirname "$0")" && pwd)
exec python3 "$here/show_hint.py" "$1" "$here/repro.py"
