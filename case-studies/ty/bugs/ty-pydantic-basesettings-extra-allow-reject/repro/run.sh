#!/bin/sh
# usage: run.sh <path-to-ty> <python-environment-with-pydantic-settings>
# e.g.   python3.13 -m venv env && env/bin/pip install pydantic-settings && ./run.sh ty env
# exit 1: ty reports unknown-argument for the extra keyword (bug present); 0: it does not.
here=$(cd "$(dirname "$0")" && pwd)
out=$("$1" check --python "$2" --python-version 3.13 --output-format concise "$here/repro.py" 2>&1)
printf '%s\n' "$out"
case $out in *unknown-argument*) exit 1 ;; *) exit 0 ;; esac
