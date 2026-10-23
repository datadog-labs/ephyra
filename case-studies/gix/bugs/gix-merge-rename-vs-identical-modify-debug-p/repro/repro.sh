#!/bin/bash
# Repro: `main` changes `a` and renames it to `c`; `side` makes the same change to `a` in place.
# git merges cleanly (c = the new content). A debug build of gix panics on a debug assertion in gix-merge.
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces (gix panics or disagrees with git); 0: gix agrees with git; 2: setup problem.
set -u
GIX_SRC=$(cd "${1:?usage: repro.sh <gitoxide-checkout>}" && pwd) || exit 2
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp -r "$HERE/merge" "$WORK/merge"
sed -i "s|GIX_PATH|$GIX_SRC/gix|" "$WORK/merge/Cargo.toml"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/target}"
cargo build -q --manifest-path "$WORK/merge/Cargo.toml" || exit 2   # debug profile: debug assertions on
BIN="$CARGO_TARGET_DIR/debug/merge"

cd "$WORK"
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
export GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'
git init -q -b main repo && cd repo || exit 2
seq 5 > a && git add a && git commit -q -m base
git checkout -q -b side
seq 6 > a && git commit -q -am theirs        # side: change a in place
git checkout -q main
git mv a c && seq 6 > c && git commit -q -am ours   # main: rename a -> c, and the same change

echo "== git merge-tree main side"
expected=$(git merge-tree --write-tree main side); git_status=$?
git ls-tree "$expected"
echo "git exit status: $git_status (0 = clean)"
[ "$git_status" = 0 ] || exit 2
echo "== gix merge_commits(main, side)"
out=$("$BIN"); gix_status=$?
echo "$out"
echo "gix exit status: $gix_status"
[ "$gix_status" = 101 ] && exit 1   # Rust panic
[ "$gix_status" = 0 ] || exit 2
read -r _ actual _ unresolved _ <<<"$out"
git ls-tree "$actual"
[ "$actual" = "$expected" ] && [ "$unresolved" = false ] && exit 0 || exit 1
