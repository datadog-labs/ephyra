#!/bin/bash
# gix status misses a gitlink whose nested repo moved, when the path is not in .gitmodules.
# Usage: repro.sh <path-to-gix-binary>   (default: gix on PATH)
# Exit 1: bug reproduces (git says modified, gix says nothing); 0: gix agrees; 2: setup failed.
GIX=${1:-gix}
T=$(mktemp -d) && cd "$T" || exit 2
export GIT_CONFIG_NOSYSTEM=1 HOME="$T" GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
git init -q repo && cd repo || exit 2
git init -q d && git -C d commit -q --allow-empty -m one || exit 2
git add d 2>/dev/null && git commit -q -m add || exit 2
git -C d commit -q --allow-empty -m two || exit 2
echo "git status --short:"; git status --short | sed 's/^/  /'
echo "gix status:";        "$GIX" status 2>&1 | sed 's/^/  /'
g=$(git status --porcelain)
x=$("$GIX" status 2>/dev/null)
[ -n "$g" ] || exit 2
[ -z "$x" ] && exit 1
exit 0
