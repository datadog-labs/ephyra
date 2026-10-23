#!/bin/bash
# Repro: gix's `Commit::describe()` ignores an annotated tag whose target is another tag.
# `git tag -a v2 v1` creates such a tag-of-tag; `git describe` picks v2 (newer), gix picks v1.
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces (gix disagrees with git); 0: gix agrees with git; 2: setup problem.
set -u
GIX_SRC=$(cd "${1:?usage: repro.sh <gitoxide-checkout>}" && pwd) || exit 2
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp -r "$HERE/describe" "$WORK/describe"
sed -i "s|GIX_PATH|$GIX_SRC/gix|" "$WORK/describe/Cargo.toml"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/target}"
cargo build -q --manifest-path "$WORK/describe/Cargo.toml" || exit 2
BIN="$CARGO_TARGET_DIR/debug/describe"

cd "$WORK"
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
git init -q -b main repo && cd repo || exit 2
export GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'
git commit -q --allow-empty -m one
export GIT_COMMITTER_DATE='1700003600 +0000'
git tag -a -m v1 v1 HEAD
export GIT_COMMITTER_DATE='1700007200 +0000'
git -c advice.nestedTag=false tag -a -m v2 v2 v1        # v2 is an annotated tag of the tag v1

expected=$(git describe) || exit 2
actual=$("$BIN") || exit 2
echo "git describe: $expected"
echo "gix describe: $actual"
[ "$actual" = "Some(\"$expected\")" ] && exit 0 || exit 1
