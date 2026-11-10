#!/bin/bash
# Repro: gix fails to resolve `@{push}` when the branch has no configured remote,
# although git falls back to `origin`.
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces (git resolves, gix errors); 0: gix agrees with git; 2: setup problem.
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
git config remote.origin.url ../origin.git
git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'
git config push.default current
git commit -q --allow-empty -m init
git update-ref refs/remotes/origin/main HEAD
# Note: no branch.main.remote, branch.main.pushRemote or remote.pushDefault.

echo "git:"
git rev-parse --symbolic-full-name '@{push}' 'main@{push}'
git rev-parse --verify -q '@{push}' >/dev/null || { echo "git itself does not resolve @{push}"; exit 2; }
echo "gix:"
"$BIN" . '@{push}' 'main@{push}' && exit 0 || exit 1
