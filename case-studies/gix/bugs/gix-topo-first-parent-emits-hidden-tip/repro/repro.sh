#!/bin/bash
# gix's topological commit walk with first-parent mode returns a tip that the end commit hides.
#   git rev-list --date-order --first-parent side ^main   # prints nothing
#   gix topo walk, tips=[side], ends=[main], Parents::First  # prints side
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces; 0: gix agrees with git; 2: setup problem.
set -u
GIX_SRC=$(cd "${1:?usage: repro.sh <gitoxide-checkout>}" && pwd) || exit 2
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp -r "$HERE/topo" "$WORK/topo"
sed -i "s|GIX_PATH|$GIX_SRC/gix|" "$WORK/topo/Cargo.toml"
cp "$GIX_SRC/Cargo.lock" "$WORK/topo/" 2>/dev/null
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/target}"
cargo build -q --manifest-path "$WORK/topo/Cargo.toml" || exit 2
BIN="$CARGO_TARGET_DIR/debug/topo"

export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
export GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'
cd "$WORK" && git init -q -b main repo && cd repo || exit 2
git commit -q --allow-empty -m base
git checkout -q -b side && git commit -q --allow-empty -m side
git checkout -q main && git merge -q --no-ff -m merge side || exit 2

status=0
for sort in date topo; do
for mode in first-parent all; do
  flag=; [ $mode = first-parent ] && flag=--first-parent
  g=$(git rev-list --$sort-order $flag side ^main | git name-rev --name-only --annotate-stdin | tr '\n' ' ')
  x=$("$BIN" . $(git rev-parse side main) $mode $sort | git name-rev --name-only --annotate-stdin | tr '\n' ' ') || exit 2
  echo "git rev-list --$sort-order $flag side ^main"
  echo "  # git: [${g% }]"
  echo "  # gix: [${x% }]"
  [ "$g" = "$x" ] || status=1
done
done
exit $status
