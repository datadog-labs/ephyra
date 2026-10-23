#!/bin/bash
# Repro: `@{-1}` names a deleted branch `origin`; the only remaining DWIM match for `origin` is the dangling
# symref refs/remotes/origin/HEAD -> refs/remotes/origin/main (missing). git skips the dangling ref and fails
# ("Needed a single revision"); gix silently resolves to the commit HEAD pointed at when leaving `origin`
# (the HEAD reflog's old id).
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces (gix resolves @{-1} although git does not); 0: gix fails like git; 2: setup problem.
set -u
GIX_SRC=$(cd "${1:?usage: repro.sh <gitoxide-checkout>}" && pwd) || exit 2
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp -r "$HERE/revparse" "$WORK/revparse"
sed -i "s|GIX_PATH|$GIX_SRC/gix|" "$WORK/revparse/Cargo.toml"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/target}" \
  cargo build -q --manifest-path "$WORK/revparse/Cargo.toml" || exit 2
BIN="${CARGO_TARGET_DIR:-$HERE/target}/debug/revparse"

cd "$WORK"
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
export GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'
git init -q -b main repo && cd repo || exit 2
git symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/main   # dangling
git commit -q --allow-empty -m init || exit 2
git checkout -q -b origin || exit 2
git checkout -q main || exit 2
git branch -q -D origin || exit 2

echo "git:"
if git rev-parse --verify '@{-1}'; then echo "git resolves @{-1}; cannot decide"; exit 2; fi
echo "gix:"
"$BIN" . '@{-1}'
case $? in
  0) exit 1 ;;   # gix returned an object
  1) exit 0 ;;   # gix reported an error, like git
  *) exit 2 ;;
esac
