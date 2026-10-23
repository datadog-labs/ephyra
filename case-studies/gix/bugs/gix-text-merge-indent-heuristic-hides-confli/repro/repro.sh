#!/bin/bash
# Repro: both sides append different lines at the end of the same file.
# git merge-file reports a conflict; `gix merge file` merges cleanly and silently combines both insertions.
# Usage: ./repro.sh <path-to-gix-binary> [path-to-git]
#   (build gix from a gitoxide checkout with: cargo build --bin gix)
# Exit 1: bug reproduces (gix merges cleanly where git conflicts); 0: gix agrees with git; 2: setup problem.
set -u
GIX=$(realpath "${1:?usage: repro.sh <gix-binary> [git]}") || exit 2
GIT=${2:-git}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK" || exit 2
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
"$GIT" init -q . || exit 2

printf 'a\n\n\nb\n'       > base
printf 'a\n\n\nb\n\nb\n'  > ours     # appends "", "b"
printf 'a\n\n\nb\nc\n'    > theirs   # appends "c"

echo "== git merge-file -p ours base theirs"
"$GIT" merge-file -p ours base theirs; git_status=$?
echo "git exit status: $git_status (1 = one conflict)"
echo "== gix merge file ours base theirs"
"$GIX" merge file ours base theirs; gix_status=$?
echo "gix exit status: $gix_status"
[ "$git_status" = 1 ] || exit 2
[ "$gix_status" = 0 ] && exit 1 || exit 0
