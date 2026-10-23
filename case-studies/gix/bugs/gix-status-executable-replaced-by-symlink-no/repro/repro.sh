#!/bin/bash
# Repro: a committed executable file `a` is replaced in the worktree by a symlink.
# git status reports a type change (`T a`); gix status reports a modification (`Modified a`).
# Usage: ./repro.sh <path-to-gitoxide-checkout> [git binary]
# Exit 1: bug reproduces (gix does not report TypeChange); 0: gix agrees with git; 2: setup problem.
set -u
GIX_SRC=$(cd "${1:?usage: repro.sh <gitoxide-checkout> [git]}" && pwd) || exit 2
GIT=${2:-git}
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp -r "$HERE/status" "$WORK/status"
sed -i "s|GIX_PATH|$GIX_SRC/gix|" "$WORK/status/Cargo.toml"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/target}"
cargo build -q --manifest-path "$WORK/status/Cargo.toml" || exit 2
BIN="$CARGO_TARGET_DIR/debug/status"

export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@a GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@a
cd "$WORK" && "$GIT" init -q -b main repo && cd repo || exit 2
echo 1 > a && chmod +x a
"$GIT" add a && "$GIT" commit -q -m init || exit 2
rm a && ln -s b a
"$GIT" --version
echo "--- git status --short"
"$GIT" status --short || exit 2
echo "--- gix status (index/worktree items)"
out=$("$BIN" .) || exit 2
echo "$out"
[ "$out" = "TypeChange a" ] && exit 0 || exit 1
