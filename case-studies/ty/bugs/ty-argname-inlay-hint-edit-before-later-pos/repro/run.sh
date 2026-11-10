#!/bin/sh
# usage: run.sh <path-to-ty>
# Starts `<ty> server`, requests the inlay hints of repro.py, applies their text edits (what an
# editor does on double-click) and compiles the result with python3.
# exit 1: the edited code no longer compiles (bug present); 0: it compiles.
here=$(cd "$(dirname "$0")" && pwd)
exec python3 "$here/show_hint.py" "$1" "$here/repro.py"
