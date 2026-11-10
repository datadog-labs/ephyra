#!/bin/bash
# Repro: both sides add the same new file `b` with the same content; only `side` makes it executable.
# git keeps `b` (add/add conflict on the mode); gix's merged tree has no `b` at all and records an Unknown conflict.
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces (`b` missing from gix's tree); 0: `b` is present; 2: setup problem.
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
echo 1 > a && git add a && git commit -q -m base
git checkout -q -b side
echo 2 > b && chmod +x b && git add b && git commit -q -m theirs   # side: adds b, executable
git checkout -q main
echo 2 > b && git add b && git commit -q -m ours                    # main: adds the same b, not executable

echo "== git merge-tree main side"
git merge-tree --write-tree main side; echo "git exit status: $? (1 = conflicts)"
echo "== gix merge_commits(main, side)"
read -r _ actual _ unresolved conflicts < <("$BIN") || exit 2
git ls-tree "$actual"
echo "gix unresolved_conflicts: $unresolved, resolutions: $conflicts"
git ls-tree --name-only "$actual" | grep -qx b && exit 0 || exit 1
