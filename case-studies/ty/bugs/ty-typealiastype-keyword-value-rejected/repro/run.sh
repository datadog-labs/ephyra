#!/bin/sh
# Usage: run.sh [path-to-ty]   (default: ty on PATH)
# Exit 1 if ty reports invalid-type-alias-type on repro.py (bug present), 0 if not, 2 if undecidable.
ty=${1:-ty}
cd "$(dirname "$0")" || exit 2
"$ty" version
out=$("$ty" check --python-version 3.13 --output-format concise repro.py 2>&1)
rc=$?
printf '%s\n' "$out"
case $rc in 0|1) ;; *) exit 2 ;; esac
if printf '%s' "$out" | grep -q 'invalid-type-alias-type'; then exit 1; fi
exit 0
