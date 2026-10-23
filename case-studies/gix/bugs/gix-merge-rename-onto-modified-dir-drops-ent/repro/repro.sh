#!/bin/bash
# Repro: one side modifies `d/f`, the other side deletes directory `d` and renames file `a` to `d`.
# `git merge-tree` keeps the modified `d/f` and moves the renamed file to `d~theirs` (conflict).
# `gix merge commit` loses the renamed file: it is neither at `d` nor at `d~theirs`.
# Pass `swap` as second argument to merge in the other direction: then gix keeps `d` (the renamed file)
# and loses the modified `d/f`.
# Usage: ./repro.sh <path-to-gix-binary> [swap]
#   (build it from a gitoxide checkout with `cargo build --release --bin gix`, then pass target/release/gix)
# Set GIT=/path/to/git to choose the git used as reference (default: git on PATH).
# Exit 1: bug reproduces (gix's tree differs from git's); 0: gix agrees with git; 2: setup problem.
set -u
GIX=$(realpath "${1:?usage: repro.sh <gix-binary> [swap]}") || exit 2
GIT=${GIT:-git}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK" || exit 2
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
export GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'

"$GIT" init -q -b main repo && cd repo || exit 2
mkdir d && echo 1 > d/f && echo a > a && "$GIT" add . && "$GIT" commit -q -m base
"$GIT" checkout -q -b ours
echo 2 > d/f && "$GIT" commit -q -am "modify d/f"
"$GIT" checkout -q -b theirs main
"$GIT" rm -q -r d && "$GIT" mv a d && "$GIT" commit -q -m "rm -r d; mv a d"

if [ "${2:-}" = swap ]; then A=theirs B=ours; else A=ours B=theirs; fi
git_tree=$("$GIT" merge-tree --write-tree "$A" "$B" | head -1)
gix_tree=$("$GIX" merge commit "$A" "$B" 2>/dev/null | cut -d' ' -f1)
[ -n "$gix_tree" ] || exit 2

echo "git merge-tree --write-tree $A $B:"; "$GIT" ls-tree -r "$git_tree"
echo "gix merge commit $A $B:";            "$GIT" ls-tree -r "$gix_tree"
[ "$gix_tree" = "$git_tree" ] && exit 0 || exit 1
