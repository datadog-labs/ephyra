#!/bin/bash
# Repro: `@{-1}` whose previous checkout name (`main`) now names an annotated tag resolves to the
# peeled commit in gix. git resolves `@{-1}` exactly like `main`, i.e. to the tag object.
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces (gix disagrees with git); 0: gix agrees with git; 2: setup problem.
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
git commit -q --allow-empty -m init
git checkout -q -b topic
git branch -q -D main
git tag -a -m t main HEAD

SPECS=('@{-1}' 'main' '@{-1}^{}')
echo "git:"
expected=""
for s in "${SPECS[@]}"; do
  id=$(git rev-parse --verify -q "$s") || { echo "git itself does not resolve $s"; exit 2; }
  echo "$s: $id ($(git cat-file -t "$id"))"
  expected+="$s: $id"$'\n'
done
echo "gix:"
actual=$("$BIN" . "${SPECS[@]}")
while read -r line; do
  id=${line##* }
  echo "$line ($(git cat-file -t "$id" 2>/dev/null || echo '?'))"
done <<<"$actual"
[ "$actual"$'\n' = "$expected" ] && exit 0 || exit 1
