#!/usr/bin/env bash
# Usage: repro.sh <path-to-brush> [<path-to-bash>]
# A backslash from an unquoted expansion should escape the next character in every
# path component of a glob word. brush keeps it in components that have no glob
# characters. Exit 1 = brush shows the bug, 0 = brush matches bash 5.3, 2 = unclear.
BRUSH=${1:?usage: repro.sh <brush> [<bash>]}
BASH_REF=${2:-}
T=$(mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT
SCRIPT='mkdir sub; touch sub/file
v='\''s\ub'\''
echo $v/*
echo $v*/*'
run() { (cd "$T" && rm -rf sub && "$1" --norc --noprofile -c "$SCRIPT" 2>&1); }
if [ -n "$BASH_REF" ]; then echo "bash:";  run "$BASH_REF"; fi
out=$(run "$BRUSH")
echo "brush:"; echo "$out"
case "$out" in
  $'sub/file\nsub/file') exit 0 ;;
  $'s\\ub/*\nsub/file') exit 1 ;;
  *) exit 2 ;;
esac
