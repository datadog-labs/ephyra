#!/bin/bash
# Repro: `gix merge commit` reports a conflict and moves the new file `d` to `d~main` when one side replaces the
# directory `d` with a file and the other side only deletes `d/f`. `git merge-tree` merges cleanly: the file `d`.
# Pass `swap` as second argument to merge in the other direction (same result: gix again writes `d~main`).
# Usage: ./repro.sh <path-to-gix-binary> [swap]
#   (build it from a gitoxide checkout with `cargo build --release --bin gix`, then pass target/release/gix)
# Exit 1: bug reproduces (gix's tree differs from git's); 0: gix agrees with git; 2: setup problem.
set -u
GIX=$(realpath "${1:?usage: repro.sh <gix-binary> [swap]}") || exit 2
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK" || exit 2
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
export GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'

git init -q -b main repo && cd repo || exit 2
mkdir d && echo c > d/c && echo f > d/f && git add d && git commit -q -m base
git branch theirs
git rm -q -r d && echo x > d && git add d && git commit -q -m "replace directory d with a file"
git checkout -q theirs
git rm -q d/f && git commit -q -m "delete d/f"
git checkout -q main

if [ "${2:-}" = swap ]; then A=theirs B=main; else A=main B=theirs; fi
git_tree=$(git merge-tree --write-tree "$A" "$B" | head -1)
gix_tree=$("$GIX" merge commit "$A" "$B" 2>/dev/null | cut -d' ' -f1)
[ -n "$gix_tree" ] || exit 2

echo "git merge-tree --write-tree $A $B:"; git ls-tree "$git_tree"
echo "gix merge commit $A $B:";            git ls-tree "$gix_tree"
[ "$gix_tree" = "$git_tree" ] && exit 0 || exit 1
