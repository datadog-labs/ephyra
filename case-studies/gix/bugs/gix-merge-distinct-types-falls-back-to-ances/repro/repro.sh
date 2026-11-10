#!/bin/bash
# Repro: `gix merge commit` drops both sides when one side edits a file and the other turns it into a symlink.
# `git merge-tree` keeps the symlink at `a` and the edited file at `a~theirs`; gix keeps only the base `a`.
# Pass `gitlink` as second argument to turn `a` into a gitlink instead of a symlink (same result).
# Usage: ./repro.sh <path-to-gix-binary> [gitlink]
#   (build it from a gitoxide checkout with `cargo build --release --bin gix`, then pass target/release/gix)
# Exit 1: bug reproduces (gix's tree differs from git's); 0: gix agrees with git; 2: setup problem.
set -u
GIX=$(realpath "${1:?usage: repro.sh <gix-binary> [gitlink]}") || exit 2
KIND=${2:-symlink}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK" || exit 2
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
export GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'

git init -q -b main repo && cd repo || exit 2
echo 1 > a && git add a && git commit -q -m base
git branch theirs
if [ "$KIND" = gitlink ]; then
  git rm -q a && git update-index --add --cacheinfo "160000,$(git rev-parse HEAD),a"
else
  git rm -q a && ln -s target a && git add a
fi
git commit -q -m "a becomes a $KIND"
git checkout -q theirs
echo 2 > a && git commit -q -am "edit a"
git checkout -q main

git_tree=$(git merge-tree --write-tree main theirs | head -1)
gix_tree=$("$GIX" merge commit main theirs 2>/dev/null | cut -d' ' -f1)
[ -n "$gix_tree" ] || exit 2

echo "git merge-tree --write-tree main theirs:"; git ls-tree "$git_tree"
echo "gix merge commit main theirs:";            git ls-tree "$gix_tree"
[ "$gix_tree" = "$git_tree" ] && exit 0 || exit 1
