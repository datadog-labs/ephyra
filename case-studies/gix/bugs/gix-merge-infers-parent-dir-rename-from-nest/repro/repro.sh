#!/bin/bash
# Repro: one side moves `d/e/f` to `y/z/f` (so `d` disappears), the other side adds `d/c`.
# `git merge-tree` keeps the addition at `d/c` (clean merge).
# `gix merge commit` moves it to `y/c` (also reported as clean): it treats `d` as renamed to `y`,
# although only the subdirectory `d/e` moved, to a different name `y/z`.
# Pass `swap` as second argument to merge in the other direction (same result).
# Usage: ./repro.sh <path-to-gix-binary> [swap]
#   (build it from a gitoxide checkout with `cargo build --bin gix`, then pass target/debug/gix)
# Set GIT=/path/to/git to choose the git used as reference (default: git on PATH).
# Exit 1: bug reproduces (gix's tree differs from git's); 0: gix agrees with git; 2: setup problem.
set -u
GIX=$(realpath "${1:?usage: repro.sh <gix-binary> [swap]}") || exit 2
GIT=${GIT:-git}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK" || exit 2
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null HOME="$WORK"
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
export GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'

"$GIT" init -q -b main repo && cd repo || exit 2
mkdir -p d/e && echo 1 > d/e/f && "$GIT" add . && "$GIT" commit -q -m base
"$GIT" checkout -q -b ours
mkdir y && "$GIT" mv d/e y/z && "$GIT" commit -q -m "mv d/e y/z"
"$GIT" checkout -q -b theirs main
echo x > d/c && "$GIT" add d/c && "$GIT" commit -q -m "add d/c"

if [ "${2:-}" = swap ]; then A=theirs B=ours; else A=ours B=theirs; fi
git_tree=$("$GIT" merge-tree --write-tree "$A" "$B" | head -1)
gix_tree=$("$GIX" merge commit "$A" "$B" 2>/dev/null | cut -d' ' -f1)
[ -n "$gix_tree" ] || exit 2

echo "git merge-tree --write-tree $A $B:"; "$GIT" ls-tree -r --name-only "$git_tree"
echo "gix merge commit $A $B:";            "$GIT" ls-tree -r --name-only "$gix_tree"
[ "$gix_tree" = "$git_tree" ] && exit 0 || exit 1
