#!/bin/bash
# Repro: in a SHA-256 repository gix does not find loose objects by abbreviated id,
# so `<short-hash>` and `<short-hash>~0` fail where git resolves them.
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
git init -q --object-format=sha256 -b main repo && cd repo || exit 2
git commit -q --allow-empty -m init   # one loose commit, one loose (empty) tree, nothing packed

SHORT=$(git rev-parse --short=7 HEAD) || exit 2
echo "git:"
git rev-parse "$SHORT" "$SHORT~0" "$(git rev-parse HEAD | cut -c1-40)" || { echo "git itself does not resolve"; exit 2; }
echo "gix:"
"$BIN" . "$SHORT" "$SHORT~0" "$(git rev-parse HEAD | cut -c1-40)" HEAD && exit 0 || exit 1
