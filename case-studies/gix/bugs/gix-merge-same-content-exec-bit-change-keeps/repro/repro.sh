#!/bin/bash
# Repro: both sides change `f` to the same content, only `side` also makes it executable.
# git merges cleanly (f = 100755 "2"); gix keeps the ancestor `f` (100644 "1") and reports a conflict.
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces (gix disagrees with git); 0: gix agrees with git; 2: setup problem.
set -u
GIX_SRC=$(cd "${1:?usage: repro.sh <gitoxide-checkout>}" && pwd) || exit 2
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp -r "$HERE/merge" "$WORK/merge"
sed -i "s|GIX_PATH|$GIX_SRC/gix|" "$WORK/merge/Cargo.toml"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/target}"
cargo build -q --manifest-path "$WORK/merge/Cargo.toml" || exit 2
BIN="$CARGO_TARGET_DIR/debug/merge"

cd "$WORK"
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
export GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'
git init -q -b main repo && cd repo || exit 2
echo 1 > f && git add f && git commit -q -m base
git checkout -q -b side
echo 2 > f && chmod +x f && git commit -q -am theirs   # side: new content + executable
git checkout -q main
echo 2 > f && git commit -q -am ours                    # main: same new content, mode unchanged

echo "== git merge-tree main side"
expected=$(git merge-tree --write-tree main side); git_status=$?
git ls-tree "$expected"
echo "git exit status: $git_status (0 = clean)"
echo "== gix merge_commits(main, side)"
read -r _ actual _ unresolved < <("$BIN") || exit 2
git ls-tree "$actual"
echo "gix unresolved_conflicts: $unresolved"
[ "$git_status" = 0 ] || exit 2
[ "$actual" = "$expected" ] && [ "$unresolved" = false ] && exit 0 || exit 1
