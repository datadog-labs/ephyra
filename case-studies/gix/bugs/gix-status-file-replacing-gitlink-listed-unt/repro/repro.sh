#!/bin/bash
# Repro: the index has a gitlink (mode 160000) at `f`, the worktree has a regular file `f`.
# git status shows only a type change for `f`; gix status reports `f` as a type change
# AND as untracked (Summary::Added from the directory walk).
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces (gix lists `f` as untracked); 0: gix agrees with git; 2: setup problem.
set -u
GIX_SRC=$(cd "${1:?usage: repro.sh <gitoxide-checkout>}" && pwd) || exit 2
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp -r "$HERE/status" "$WORK/status"
sed -i "s|GIX_PATH|$GIX_SRC/gix|" "$WORK/status/Cargo.toml"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/target}" \
  cargo build -q --manifest-path "$WORK/status/Cargo.toml" || exit 2
BIN="${CARGO_TARGET_DIR:-$HERE/target}/debug/status"

export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@a GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@a
cd "$WORK" && git init -q -b main repo && cd repo || exit 2
git commit -q --allow-empty -m init || exit 2
git update-index --add --cacheinfo "160000,$(git rev-parse HEAD),f" || exit 2
git commit -q -m sub || exit 2
echo x > f
echo "--- git status --porcelain=v2"
git status --porcelain=v2 || exit 2
echo "--- git ls-files -o"
git ls-files -o || exit 2
echo "--- gix status (index/worktree items)"
"$BIN" .
