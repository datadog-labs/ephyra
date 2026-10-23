#!/bin/bash
# Repro: a path that is in HEAD but whose index entry is intent-to-add
# (`git rm --cached a; git add -N a`) is a staged deletion for git, but gix's
# `Repository::status()` reports no HEAD-vs-index change for it.
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces; 0: gix agrees with git; 2: setup problem.
set -u
GIX_SRC=$(cd "${1:?usage: repro.sh <gitoxide-checkout>}" && pwd) || exit 2
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp -r "$HERE/status" "$WORK/status"
sed -i "s|GIX_PATH|$GIX_SRC/gix|" "$WORK/status/Cargo.toml"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/target}"
cargo build -q --manifest-path "$WORK/status/Cargo.toml" || exit 2
BIN="$CARGO_TARGET_DIR/debug/status"

cd "$WORK"
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
git init -q -b main repo && cd repo || exit 2
echo 1 > a
git add a && git commit -q -m init
git rm -q --cached a
git add -N a

echo "git diff --cached --name-status:"
want=$(git diff --cached --name-status | tr '\t' ' ')
echo "$want"
[ "$want" = "D a" ] || { echo "unexpected git output"; exit 2; }
echo "gix status (HEAD vs index):"
got=$("$BIN" .) || exit 2
echo "$got"
[ "$got" = "$want" ] && exit 0 || exit 1
