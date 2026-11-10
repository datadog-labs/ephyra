#!/bin/sh
# usage: run.sh <path-to-ty>
# Starts `<ty> server`, requests the inlay hints of repro.py, applies their text edits (what an
# editor does on double-click), then runs `ty check` and python3 on the original and edited source.
# exit 1: ty check accepts repro.py but rejects the edited source (bug present); 0 otherwise.
here=$(cd "$(dirname "$0")" && pwd)
exec python3 "$here/show_hint.py" "$1" "$here/repro.py"
