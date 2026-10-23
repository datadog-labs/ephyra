#!/bin/bash
# Repro: HEAD -> refs/heads/main, and refs/heads/main is itself a symbolic ref to the
# not-yet-existing refs/heads/topic. git says "On branch topic / No commits yet";
# gix's head() does not see the unborn branch, and status() fails with
# "The ref partially named "refs/heads/topic" could not be found".
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces (gix errors); 0: gix handles it like git; 2: setup problem.
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
git init -q -b main "$WORK/repo" || exit 2
git -C "$WORK/repo" symbolic-ref refs/heads/main refs/heads/topic || exit 2
echo "--- git status"
git -C "$WORK/repo" status || exit 2
echo "--- gix"
"$BIN" "$WORK/repo"
