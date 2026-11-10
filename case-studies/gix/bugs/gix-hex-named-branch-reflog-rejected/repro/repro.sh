#!/bin/bash
# Repro: a branch whose name is also an abbreviated object id (e.g. branch `4e72` where HEAD is 4e72…):
# gix resolves `4e72` to that branch, but rejects `4e72@{0}` / `4e72@{<date>}` with
# "reflog entries require a ref name", while git resolves them from the branch's reflog.
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
git init -q --object-format=sha1 -b main repo && cd repo || exit 2
git commit -q --allow-empty -m init
H=$(git rev-parse --short=4 HEAD)   # name the branch like HEAD's 4-char abbreviation
git branch "$H" HEAD
echo "branch: $H (HEAD is $(git rev-parse HEAD))"

SPECS=("$H" "$H@{0}" "$H@{2023-11-14 22:13:20 +0000}")
echo "git:"
expected=""
for s in "${SPECS[@]}"; do
  id=$(git rev-parse --verify -q "$s") || { echo "git itself does not resolve $s"; exit 2; }
  echo "$s: $id"
  expected+="$s: $id"$'\n'
done
echo "gix:"
actual=$("$BIN" . "${SPECS[@]}")
echo "$actual"
[ "$actual"$'\n' = "$expected" ] && exit 0 || exit 1
