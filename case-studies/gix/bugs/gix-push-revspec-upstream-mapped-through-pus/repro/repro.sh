#!/bin/bash
# Repro: with push.default=upstream in a triangular setup (branch.main.remote=up,
# remote.pushDefault=origin), gix resolves `@{push}` to refs/remotes/origin/main,
# while git resolves it to the upstream tracking branch refs/remotes/up/main.
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces (gix and git disagree); 0: gix agrees with git; 2: setup problem.
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
git commit -q --allow-empty -m one
git commit -q --allow-empty -m two
git config remote.origin.url ../origin.git
git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'
git config remote.up.url ../up.git
git config remote.up.fetch '+refs/heads/*:refs/remotes/up/*'
git config branch.main.remote up
git config branch.main.merge refs/heads/main
git config remote.pushDefault origin
git config push.default upstream
git update-ref refs/remotes/up/main HEAD
git update-ref refs/remotes/origin/main HEAD~1

echo "git:"
git rev-parse --symbolic-full-name '@{push}'
want=$(git rev-parse --verify -q '@{push}') || { echo "git itself does not resolve @{push}"; exit 2; }
echo "@{push}: $want"
echo "gix:"
out=$("$BIN" . '@{push}')
echo "$out"
[ "$out" = "@{push}: $want" ] && exit 0 || exit 1
