#!/usr/bin/env bash
# Usage: run.sh <python-with-torchspec>; exit 1 = bug reproduces, 0 = fixed.
cd "$(dirname "$(realpath "$0")")" || exit 3
out=$("$1" poc.py 2>&1); rc=$?
echo "$out" | tail -3
if [ $rc -ne 0 ] && echo "$out" | grep -q "^ValueError: .*dimension"; then exit 1; fi
[ $rc -eq 0 ] && exit 0; exit 2
