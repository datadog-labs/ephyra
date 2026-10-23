#!/bin/bash
# Repro: `origin@{0}` (refs/remotes/origin/HEAD@{0}) resolves to the newest entry of origin/HEAD's own
# reflog instead of the ref's current value when origin/HEAD's target moved afterwards. git returns the
# current value (and warns "log for ref refs/remotes/origin/HEAD unexpectedly ended on ...").
# The same happens for a date after the newest entry, e.g. `origin@{2030-01-01}`.
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
git init -q -b main repo && cd repo || exit 2
export GIT_AUTHOR_DATE='1700003600 +0000' GIT_COMMITTER_DATE='1700003600 +0000'
git commit -q --allow-empty -m one
export GIT_AUTHOR_DATE='1700007200 +0000' GIT_COMMITTER_DATE='1700007200 +0000'
git commit -q --allow-empty -m two
# Like a clone followed by a fetch: origin/HEAD gets its own reflog when created, and only the
# target's reflog is appended when origin/main moves later.
git update-ref refs/remotes/origin/main HEAD~1
git symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/main
git update-ref refs/remotes/origin/main HEAD
echo "one: $(git rev-parse HEAD~1)"
echo "two: $(git rev-parse HEAD)"
echo "origin/HEAD reflog:"; cat .git/logs/refs/remotes/origin/HEAD

SPECS=('origin' 'origin@{0}' 'origin@{2030-01-01}')
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
