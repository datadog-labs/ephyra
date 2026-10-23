#!/bin/bash
# Repro: `repo.rev_walk(tips).with_hidden(hidden)` returns an ancestor of a hidden commit
# when commit times are not monotonic and there is no commit-graph file.
# History (linear): root(t=2) <- skewed(t=0) <- visible(t=3) <- hidden(t=4)
# `git rev-list visible root ^hidden` prints nothing; gix prints `root`.
# Usage: ./repro.sh <path-to-gitoxide-checkout> [git binary]
# Exit 1: bug reproduces (gix disagrees with git); 0: gix agrees with git; 2: setup problem.
set -u
GIX_SRC=$(cd "${1:?usage: repro.sh <gitoxide-checkout> [git]}" && pwd) || exit 2
GIT=${2:-git}
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp -r "$HERE/walk" "$WORK/walk"
sed -i "s|GIX_PATH|$GIX_SRC/gix|" "$WORK/walk/Cargo.toml"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/target}"
cargo build -q --manifest-path "$WORK/walk/Cargo.toml" || exit 2
WALK="$CARGO_TARGET_DIR/debug/walk"

cd "$WORK"
export HOME="$WORK" XDG_CONFIG_HOME="$WORK" GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
"$GIT" init -q -b main repo && cd repo || exit 2
commit() {  # commit <seconds after 1700000000> <tag>
    GIT_AUTHOR_DATE="$((1700000000 + $1)) +0000" GIT_COMMITTER_DATE="$((1700000000 + $1)) +0000" \
        "$GIT" commit -q --allow-empty -m "$2" && "$GIT" tag "$2"
}
commit 2 root
commit 0 skewed     # committer time older than its parent
commit 3 visible
commit 4 hidden

names() { while read -r id; do "$GIT" describe --tags --exact-match "$id"; done | tr '\n' ' '; }
expected=$("$GIT" rev-list visible root ^hidden | names) || exit 2
actual=$("$WALK" visible root ^hidden | names) || exit 2
echo "git rev-list visible root ^hidden: [${expected% }]"
echo "gix rev_walk([visible, root]).with_hidden([hidden]): [${actual% }]"
[ "$actual" = "$expected" ] && exit 0 || exit 1
